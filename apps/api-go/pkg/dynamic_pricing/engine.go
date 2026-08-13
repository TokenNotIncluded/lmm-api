// Package dynamic_pricing implements the pure pricing engine for the
// dynamic pricing feature.
//
// Pipeline (per model, per tick):
//
//  1. routeCost = ComputeRouteCost(cheapCost, backupCost, failoverProb):
//     the expected upstream cost of serving a request, given the chance of
//     failing over to a more expensive backup channel.
//  2. CostEMA is updated from the measured upstream cost (USD per 1M tokens)
//     with an exponential moving average (AlphaLoad).
//  3. C = EffectiveCost(routeCost, CostEMA): the effective unit cost used as
//     the anchor for the factor. The cost floor is enforced here by never
//     letting C drop below the smoothed actual upstream cost.
//  4. raw = RawLoad(...) measures how far the window exceeded the TPM/RPM /
//     cost-rate targets; LoadEMA smooths it.
//  5. heat = Heat(LoadEMA, deadzone, gamma) maps excess load to [0,1] with a
//     deadzone (small fluctuations produce no heat) and a non-linear shaping
//     exponent.
//  6. target = DynamicMultiplier(C, maxFactor, heat): the desired multiplier
//     in [1, maxFactor], i.e. the supply-demand premium on top of the
//     configured price.
//  7. smoothed = NextMultiplier(...): an asymmetric EMA so the factor rises
//     fast (AlphaUp) but falls slowly (AlphaDown).
//  8. final = EnforceBounds(...): per-tick step clamps (MaxStepUp /
//     MaxStepDown) and the absolute range [1, maxFactor].
//
// Design: cost floor (the price never undercuts the upstream cost) plus a
// supply-demand premium (excess load raises the multiplier), moved with an
// asymmetric EMA and step clamps for smooth, bounded changes. Revenue is
// deliberately never used as a load input: charging more under load would
// feed back into higher measured revenue and could spiral. Only token/request
// volume and upstream cost drive the factor.
//
// All functions are pure: they take explicit inputs and return values, with
// no I/O and no access to global state. Per-model target overrides
// (setting/dynamic_pricing_setting.GetModelTargets) must be resolved by the
// caller into the DynamicPricingSetting passed to Tick; Tick itself only
// reads the setting value it is given.
package dynamic_pricing

import (
	"math"

	"github.com/QuantumNous/new-api/setting/dynamic_pricing_setting"
)

// ModelState holds the persistent per-model pricing state.
//
// The initial Factor is chosen by the caller (e.g. the setting's
// CostFloorFactor) when the state is first created; the engine only evolves
// it from there.
type ModelState struct {
	LoadEMA   float64 // smoothed load (dimensionless; 1.0 == at target)
	CostEMA   float64 // smoothed actual unit cost, USD per 1M tokens
	Factor    float64 // current dynamic multiplier
	UpdatedAt int64   // unix seconds of the last tick
}

// TickInput carries the per-tick window measurements for one model.
type TickInput struct {
	Model                 string
	WindowTokens          float64 // total tokens in the window
	WindowRequests        float64
	WindowUpstreamCostUSD float64 // sum of tokens×channel cost / 1M across the window, USD
	WindowMinutes         float64
	CheapCost             float64 // USD per 1M tokens of the cheapest route; 0 = unknown
	BackupCost            float64 // USD per 1M tokens of the failover route; 0 = unknown
	Now                   int64   // unix seconds of this tick
}

// ComputeRouteCost returns the expected upstream unit cost (USD per 1M
// tokens) of a request given the cost of the cheap route, the cost of the
// backup route, and the probability of failing over to the backup. The
// failover probability is clamped to [0,1]. When the backup cost is unknown
// (<= 0) or not more expensive than the cheap route, the cheap cost is used
// as-is.
func ComputeRouteCost(cheapCost, backupCost, failoverProb float64) float64 {
	if backupCost <= 0 || backupCost < cheapCost {
		return cheapCost
	}
	p := math.Max(0, math.Min(1, failoverProb))
	return cheapCost + p*(backupCost-cheapCost)
}

// EffectiveCost returns the effective unit cost C anchoring the factor:
// the larger of the expected route cost and the smoothed actual upstream
// cost, so C never undercuts what we actually pay (cost floor). floorFactor
// is part of the engine's public signature; the floor itself is realized by
// this max, and the factor-level bounds are applied in EnforceBounds.
func EffectiveCost(routeCost, costEMA, floorFactor float64) float64 {
	return math.Max(routeCost, costEMA)
}

// RawLoad measures how far the window exceeded the configured targets,
// returning the maximum over the dimensions whose target is positive:
//
//	tpm      = tokens      / windowMinutes / targetTPM
//	rpm      = requests    / windowMinutes / targetRPM
//	costRate = costUSD     / windowMinutes / targetCostRate
//
// A value of 1.0 means "exactly at target"; values above 1.0 mean the
// window was overloaded. If all targets are <= 0, or the window length is
// not positive, it returns 0 (no load signal).
func RawLoad(tokens, requests, costUSD, windowMinutes, targetTPM, targetRPM, targetCostRate float64) float64 {
	if windowMinutes <= 0 {
		return 0
	}
	raw := 0.0
	if targetTPM > 0 {
		raw = math.Max(raw, tokens/windowMinutes/targetTPM)
	}
	if targetRPM > 0 {
		raw = math.Max(raw, requests/windowMinutes/targetRPM)
	}
	if targetCostRate > 0 {
		raw = math.Max(raw, costUSD/windowMinutes/targetCostRate)
	}
	return raw
}

// UpdateLoadEMA smooths a sample into an exponential moving average. On a
// cold start (prev == 0) the raw sample is taken directly so the first tick
// reflects reality instead of decaying toward zero.
func UpdateLoadEMA(prev, raw, alpha float64) float64 {
	if prev == 0 {
		return raw
	}
	return prev + alpha*(raw-prev)
}

// Heat maps excess load to [0,1] through a deadzone and a non-linear
// shaping exponent:
//
//	x = clamp((load-deadzone)/(1-deadzone), 0, 1)
//	heat = x^gamma
//
// Load at or below the deadzone produces no heat; a gamma greater than 1
// keeps small excesses gentle and amplifies large ones. Invalid deadzones
// (outside [0,1)) default to 0; gamma below 1 defaults to 1.
func Heat(load, deadzone, gamma float64) float64 {
	if deadzone < 0 || deadzone >= 1 {
		deadzone = 0
	}
	if gamma < 1 {
		gamma = 1
	}
	x := (load - deadzone) / (1 - deadzone)
	x = math.Max(0, math.Min(1, x))
	return math.Pow(x, gamma)
}

// DynamicMultiplier turns the effective cost C and the current heat into the
// desired multiplier applied to the configured price: 1 + (maxFactor-1)×heat.
// With heat in [0,1] the result is in [1, maxFactor]. If the cost is unknown
// or not positive (feature off for this model), it returns 1.0 (base price).
func DynamicMultiplier(cost float64, maxFactor, heat float64) float64 {
	if cost <= 0 {
		return 1.0
	}
	if maxFactor < 1 {
		maxFactor = 1
	}
	return 1 + (maxFactor-1)*heat
}

// EnforceBounds applies the per-tick movement clamps and the absolute range
// to a candidate multiplier. It clamps mult to at most prev×(1+maxStepUp)
// and at least prev×(1-maxStepDown); a non-positive prev means the first
// tick, where no step clamp is applied. The result is then clamped into
// [1.0, maxFactor]. If the cost is unknown or not positive, it returns 1.0.
// floorFactor is part of the engine's public signature; the cost floor is
// enforced upstream via EffectiveCost.
func EnforceBounds(mult, prevMult float64, cost float64, floorFactor, maxFactor, maxStepUp, maxStepDown float64) float64 {
	if cost <= 0 {
		return 1.0
	}
	if maxFactor < 1 {
		maxFactor = 1
	}
	if prevMult > 0 {
		if mult > prevMult*(1+maxStepUp) {
			mult = prevMult * (1 + maxStepUp)
		}
		if mult < prevMult*(1-maxStepDown) {
			mult = prevMult * (1 - maxStepDown)
		}
	}
	return math.Max(1.0, math.Min(maxFactor, mult))
}

// NextMultiplier applies an asymmetric EMA toward the target multiplier:
// AlphaUp when the factor must rise, AlphaDown when it must fall. A
// non-positive prev is a cold start and returns the target directly.
func NextMultiplier(prev float64, target float64, alphaUp, alphaDown float64) float64 {
	if prev <= 0 {
		return target
	}
	alpha := alphaDown
	if target > prev {
		alpha = alphaUp
	}
	return prev + alpha*(target-prev)
}

// Tick orchestrates one pricing tick for one model and returns the new
// multiplier. It updates the fields of state in place (CostEMA, LoadEMA,
// Factor, UpdatedAt).
//
// The caller is responsible for building TickInput from its window
// accumulator and for resolving per-model target overrides into the setting
// it passes. If the raw load exceeds 1.0 the caller may log a warning; the
// engine only computes.
//
// No-cost-signal ticks: when this tick carries no upstream cost information
// at all (CheapCost <= 0 AND BackupCost <= 0 AND WindowUpstreamCostUSD <= 0),
// the model is treated as having lost its cost configuration (e.g. the admin
// removed the channel_costs entries mid-run). The state is reset to the base
// price immediately: CostEMA and Factor are cleared to 0/1.0, UpdatedAt is
// set, and 1.0 is returned without evolving the EMA. This is deliberate: a
// stale warm CostEMA would otherwise keep the multiplier elevated even though
// the feature no longer has any cost signal for this model, so prices must
// fall back to the configured base.
func Tick(state *ModelState, in TickInput, s dynamic_pricing_setting.DynamicPricingSetting) float64 {
	if state == nil {
		return 1.0
	}

	// No cost signal this tick: admin config-change scenario, fall back to
	// base price immediately (see doc comment above).
	if in.CheapCost <= 0 && in.BackupCost <= 0 && in.WindowUpstreamCostUSD <= 0 {
		state.CostEMA = 0
		state.Factor = 1.0
		state.UpdatedAt = in.Now
		return 1.0
	}

	// Defensive cold-start guard: a Factor <= 0 would make NextMultiplier
	// return the target directly and EnforceBounds skip the step clamp, so a
	// hot first tick could jump straight to maxFactor. Treat it as 1.0 so the
	// step clamp binds from the very first tick. (The service layer also
	// seeds fresh states with Factor 1.0; this guard covers pure-function
	// callers that pass an empty ModelState.)
	if state.Factor <= 0 {
		state.Factor = 1.0
	}

	// (a,b) expected upstream cost given failover routing.
	routeCost := ComputeRouteCost(in.CheapCost, in.BackupCost, s.FailoverProbability)

	// (c,d) measured unit cost, USD per 1M tokens; keep the previous EMA if
	// the window carried no tokens.
	var actualUnitCost float64
	if in.WindowTokens > 0 {
		actualUnitCost = in.WindowUpstreamCostUSD * 1e6 / in.WindowTokens
	}
	if actualUnitCost > 0 {
		state.CostEMA = UpdateLoadEMA(state.CostEMA, actualUnitCost, s.AlphaLoad)
	}

	// (e) effective cost: never below the smoothed actual upstream cost.
	C := EffectiveCost(routeCost, state.CostEMA, s.CostFloorFactor)

	// (f) load vs targets, smoothed.
	raw := RawLoad(in.WindowTokens, in.WindowRequests, in.WindowUpstreamCostUSD,
		in.WindowMinutes, s.TargetTPM, s.TargetRPM, s.TargetCostRate)
	state.LoadEMA = UpdateLoadEMA(state.LoadEMA, raw, s.AlphaLoad)

	// (g,h) heat and desired multiplier.
	heat := Heat(state.LoadEMA, s.LoadDeadzone, s.HeatGamma)
	target := DynamicMultiplier(C, s.MaxFactor, heat)

	// (i,j) asymmetric EMA plus step clamps and absolute bounds.
	smoothed := NextMultiplier(state.Factor, target, s.AlphaUp, s.AlphaDown)
	final := EnforceBounds(smoothed, state.Factor, C, s.CostFloorFactor,
		s.MaxFactor, s.MaxStepUp, s.MaxStepDown)

	// (k) persist and return.
	state.Factor = final
	state.UpdatedAt = in.Now
	return final
}
