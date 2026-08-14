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
//  3. C = EffectiveCost(max(routeCost, currentCost), CostEMA): the effective
//     unit cost used as the anchor for the factor, including the configured
//     cost-floor factor. The current measured cost is applied immediately.
//  4. raw = RawLoad(...) measures how far the window exceeded the TPM/RPM /
//     cost-rate targets; LoadEMA smooths it.
//  5. heat = Heat(LoadEMA, deadzone, gamma) maps excess load to [0,1] with a
//     deadzone (small fluctuations produce no heat) and a non-linear shaping
//     exponent.
//  6. target = DynamicMultiplier(C, basePrice, maxFactor, heat): the bounded
//     demand target in [1, maxFactor].
//  7. smoothed = NextMultiplier(...): an asymmetric EMA so the factor rises
//     fast (AlphaUp) but falls slowly (AlphaDown).
//  8. final = max(MinFactor, CostFloorMultiplier(...), EnforceBounds(...)).
//     Step clamps and maxFactor apply only to the demand premium; known-cost
//     and operator floors are immediate and may exceed that demand ceiling.
//
// Design: cost floor (the price never undercuts the upstream cost scaled by
// CostFloorFactor) plus a supply-demand premium (excess load raises the
// multiplier), moved with an asymmetric EMA and step clamps for smooth,
// bounded changes. Revenue is deliberately never used as a load input:
// charging more under load would feed back into higher measured revenue and
// could spiral. Only token/request volume and upstream cost drive the factor.
//
// All functions are pure: they take explicit inputs and return values, with
// no I/O and no access to global state. Per-model target overrides
// (setting/dynamic_pricing_setting.GetModelTargets) must be resolved by the
// caller into the DynamicPricingSetting passed to Tick; Tick itself only
// reads the setting value it is given.
package dynamic_pricing

import (
	"math"

	"github.com/LIghtJUNction/api.lmm.best/setting/dynamic_pricing_setting"
)

// ModelState holds the persistent per-model pricing state.
//
// The initial Factor is chosen by the caller (normally MinFactor) when the
// state is first created; the engine only evolves it from there.
type ModelState struct {
	LoadEMA            float64 // smoothed load (dimensionless; 1.0 == at target)
	CostEMA            float64 // smoothed actual unit cost, USD per 1M tokens
	Factor             float64 // current effective dynamic multiplier
	CostFloor          float64 // immediate configured-cost floor multiplier
	UnpricedTokens     float64 // latest-window tokens on channels without a cost
	UnpricedRequests   float64 // latest-window requests on channels without a cost
	HasUnpricedTraffic bool    // latest window contains traffic with unknown cost
	UpdatedAt          int64   // unix seconds of the last tick
}

// TickInput carries the per-tick window measurements for one model.
type TickInput struct {
	Model                  string
	WindowTokens           float64 // priced tokens in the window
	WindowRequests         float64 // requests on priced channels
	WindowUpstreamCostUSD  float64 // sum of tokens×channel cost / 1M across the window, USD
	WindowMinutes          float64
	CheapCost              float64 // USD per 1M tokens of the cheapest route; 0 = unknown
	BackupCost             float64 // USD per 1M tokens of the failover route; 0 = unknown
	WindowUnpricedTokens   float64 // traffic on channels without a configured cost
	WindowUnpricedRequests float64 // requests on channels without a configured cost
	BasePriceUSDPerMillion float64 // positive reference price used by the cost floor
	Now                    int64   // unix seconds of this tick
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

// EffectiveCost returns the effective unit cost C anchoring the factor: the
// larger of the expected route cost and the smoothed actual upstream cost,
// scaled by floorFactor. Costs are USD per 1M tokens. A non-positive or
// non-finite floorFactor is treated as 1.0 so pure callers fail safe.
func EffectiveCost(routeCost, costEMA, floorFactor float64) float64 {
	anchor := 0.0
	if isFinitePositive(routeCost) {
		anchor = math.Max(anchor, routeCost)
	}
	if isFinitePositive(costEMA) {
		anchor = math.Max(anchor, costEMA)
	}
	if anchor <= 0 {
		return 0
	}
	if !isFinitePositive(floorFactor) || floorFactor < 1 {
		floorFactor = 1
	}
	return anchor * floorFactor
}

// CostFloorMultiplier converts a conservative upstream unit cost into the
// request multiplier needed to cover it, including the configured safety
// margin. Unlike the load premium, this hard floor is not capped by
// max_factor: a price ceiling must never force the selling price below known
// cost. Invalid or unknown inputs return zero so callers can fail closed.
func CostFloorMultiplier(unitCost, basePrice, floorFactor float64) float64 {
	if !isFinitePositive(unitCost) || !isFinitePositive(basePrice) {
		return 0
	}
	if !isFinitePositive(floorFactor) || floorFactor < 1 {
		floorFactor = 1
	}
	floor := unitCost * floorFactor / basePrice
	if !isFinitePositive(floor) {
		return 0
	}
	return math.Max(1, floor)
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

// DynamicMultiplier turns the effective cost C and current heat into the
// desired multiplier applied to the configured base price. The target is the
// larger of the cost-floor multiplier C/basePrice and the load premium
// 1+(maxFactor-1)×heat, capped at maxFactor. A non-positive cost produces the
// neutral multiplier; a non-positive base price falls back to 1 USD per 1M.
func DynamicMultiplier(cost, basePrice float64, maxFactor, heat float64) float64 {
	if !isFinitePositive(cost) {
		return 1.0
	}
	if !isFinitePositive(basePrice) {
		basePrice = 1.0
	}
	if maxFactor < 1 {
		maxFactor = 1
	}
	if !isFinite(heat) {
		heat = 0
	}
	heat = math.Max(0, math.Min(1, heat))
	costMultiplier := cost / basePrice
	loadMultiplier := 1 + (maxFactor-1)*heat
	return math.Max(1, math.Min(maxFactor, math.Max(costMultiplier, loadMultiplier)))
}

// EnforceBounds applies the per-tick movement clamps and the absolute range
// to a candidate multiplier. It clamps mult to at most prev×(1+maxStepUp)
// and at least prev×(1-maxStepDown); a non-positive prev means the first
// tick, where no step clamp is applied. The result is then clamped into
// [1.0, maxFactor]. If the cost is unknown or not positive, it returns 1.0.
// floorFactor remains in the signature for callers that build the full
// pipeline; EffectiveCost has already applied it before this function runs.
func EnforceBounds(mult, prevMult float64, cost float64, floorFactor, maxFactor, maxStepUp, maxStepDown float64) float64 {
	if !isFinitePositive(cost) {
		return 1.0
	}
	if !isFinite(mult) {
		mult = 1.0
	}
	if maxFactor < 1 {
		maxFactor = 1
	}
	if !isFinite(maxStepUp) || maxStepUp < 0 {
		maxStepUp = 0
	}
	if !isFinite(maxStepDown) || maxStepDown < 0 {
		maxStepDown = 0
	}
	if maxStepUp > 1 {
		maxStepUp = 1
	}
	if maxStepDown > 1 {
		maxStepDown = 1
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

// ClampState sanitizes state loaded from Redis or supplied by an integration
// boundary. Invalid EMA values are cleared, and Factor is always kept inside
// [1, maxFactor]. It intentionally does not alter UpdatedAt.
func ClampState(state *ModelState, maxFactor float64) {
	if state == nil {
		return
	}
	if !isFinite(state.LoadEMA) || state.LoadEMA < 0 {
		state.LoadEMA = 0
	}
	if !isFinite(state.CostEMA) || state.CostEMA < 0 {
		state.CostEMA = 0
	}
	if !isFinite(state.CostFloor) || state.CostFloor < 0 {
		state.CostFloor = 0
	}
	if !isFinite(state.UnpricedTokens) || state.UnpricedTokens < 0 {
		state.UnpricedTokens = 0
	}
	if !isFinite(state.UnpricedRequests) || state.UnpricedRequests < 0 {
		state.UnpricedRequests = 0
	}
	if !isFinitePositive(maxFactor) || maxFactor < 1 {
		maxFactor = 1
	}
	if !isFinite(state.Factor) || state.Factor < 1 {
		state.Factor = 1
	}
	// max_factor caps only the demand premium. A known-cost floor may
	// legitimately exceed it and must survive Redis restoration.
	upperBound := math.Max(maxFactor, state.CostFloor)
	if state.Factor > upperBound {
		state.Factor = upperBound
	}
}

func isFinite(value float64) bool {
	return !math.IsNaN(value) && !math.IsInf(value, 0)
}

func isFinitePositive(value float64) bool {
	return value > 0 && isFinite(value)
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
// No-cost-signal ticks with non-zero unpriced traffic reset the engine state
// to MinFactor: the model has traffic but no trustworthy cost anchor. The
// strict request path separately blocks that selected channel before spend.
// A genuinely idle model follows the decay path below so stale high factors
// do not survive zero-traffic windows indefinitely.
func Tick(state *ModelState, in TickInput, s dynamic_pricing_setting.DynamicPricingSetting) float64 {
	if state == nil {
		return 1.0
	}
	if err := s.Validate(); err != nil {
		minimum := 1.0
		if isFinitePositive(s.MinFactor) && s.MinFactor >= 1 {
			minimum = s.MinFactor
		}
		state.LoadEMA = 0
		state.CostEMA = 0
		state.CostFloor = 0
		state.UnpricedTokens = in.WindowUnpricedTokens
		state.UnpricedRequests = in.WindowUnpricedRequests
		state.HasUnpricedTraffic = in.WindowUnpricedTokens > 0 || in.WindowUnpricedRequests > 0
		state.Factor = minimum
		state.UpdatedAt = in.Now
		return minimum
	}
	ClampState(state, s.MaxFactor)
	minimum := s.MinFactor
	state.UnpricedTokens = math.Max(0, in.WindowUnpricedTokens)
	state.UnpricedRequests = math.Max(0, in.WindowUnpricedRequests)
	state.HasUnpricedTraffic = state.UnpricedTokens > 0 || state.UnpricedRequests > 0

	// No traffic is an explicit zero-load sample. Decay both EMAs and the
	// factor toward the neutral price, subject to the same downward step clamp
	// used by normal ticks. This is what makes stale high factors fall when a
	// model has no rows in the current window.
	if in.WindowTokens <= 0 && in.WindowRequests <= 0 &&
		in.WindowUnpricedTokens <= 0 && in.WindowUnpricedRequests <= 0 {
		state.LoadEMA = UpdateLoadEMA(state.LoadEMA, 0, s.AlphaLoad)
		state.CostEMA = UpdateLoadEMA(state.CostEMA, 0, s.AlphaLoad)
		state.CostFloor = 0
		state.HasUnpricedTraffic = false
		state.UnpricedTokens = 0
		state.UnpricedRequests = 0
		smoothed := NextMultiplier(state.Factor, minimum, s.AlphaUp, s.AlphaDown)
		state.Factor = EnforceBounds(smoothed, state.Factor, 1.0, s.CostFloorFactor,
			s.MaxFactor, s.MaxStepUp, s.MaxStepDown)
		state.Factor = math.Max(minimum, state.Factor)
		state.UpdatedAt = in.Now
		return state.Factor
	}

	// Non-zero traffic without a configured cost is not safe to price from. Do
	// not let a warm CostEMA keep charging a model whose only observed channels
	// are unpriced.
	if in.CheapCost <= 0 && in.BackupCost <= 0 && in.WindowUpstreamCostUSD <= 0 {
		state.CostEMA = 0
		state.CostFloor = 0
		state.Factor = minimum
		state.UpdatedAt = in.Now
		return minimum
	}

	// Defensive cold-start guard: a Factor <= 0 would make NextMultiplier
	// return the target directly and EnforceBounds skip the step clamp, so a
	// hot first tick could jump straight to maxFactor. Treat it as 1.0 so the
	// step clamp binds from the very first tick. (The service layer also
	// seeds fresh states with Factor 1.0; this guard covers pure-function
	// callers that pass an empty ModelState.)
	if state.Factor <= 0 {
		state.Factor = minimum
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

	// (e) effective cost: include the current window's actual unit cost
	// immediately. Cost increases must not wait for EMA or step-up convergence.
	currentCost := math.Max(routeCost, actualUnitCost)
	C := EffectiveCost(currentCost, state.CostEMA, s.CostFloorFactor)
	state.CostFloor = CostFloorMultiplier(
		math.Max(currentCost, state.CostEMA),
		in.BasePriceUSDPerMillion,
		s.CostFloorFactor,
	)

	// (f) load vs targets, smoothed.
	raw := RawLoad(in.WindowTokens, in.WindowRequests, in.WindowUpstreamCostUSD,
		in.WindowMinutes, s.TargetTPM, s.TargetRPM, s.TargetCostRate)
	state.LoadEMA = UpdateLoadEMA(state.LoadEMA, raw, s.AlphaLoad)

	// (g,h) heat and desired multiplier.
	heat := Heat(state.LoadEMA, s.LoadDeadzone, s.HeatGamma)
	target := DynamicMultiplier(C, in.BasePriceUSDPerMillion, s.MaxFactor, heat)

	// (i,j) asymmetric EMA plus step clamps and absolute bounds.
	smoothed := NextMultiplier(state.Factor, target, s.AlphaUp, s.AlphaDown)
	final := EnforceBounds(smoothed, state.Factor, C, s.CostFloorFactor,
		s.MaxFactor, s.MaxStepUp, s.MaxStepDown)
	// Financial safety floors are immediate and intentionally override both
	// smoothing/step clamps and max_factor. Only the demand premium is bounded.
	final = math.Max(final, minimum)
	final = math.Max(final, state.CostFloor)

	// (k) persist and return.
	state.Factor = final
	state.UpdatedAt = in.Now
	return final
}
