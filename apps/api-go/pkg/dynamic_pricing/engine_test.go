package dynamic_pricing

import (
	"math"
	"testing"

	"github.com/QuantumNous/new-api/setting/dynamic_pricing_setting"
)

// approxEqual reports whether a and b are within eps of each other.
func approxEqual(a, b, eps float64) bool {
	return math.Abs(a-b) <= eps
}

// defaultSetting returns a DynamicPricingSetting with the same defaults the
// config package registers, so Tick tests are deterministic.
func defaultSetting() dynamic_pricing_setting.DynamicPricingSetting {
	return dynamic_pricing_setting.DynamicPricingSetting{
		Enabled:             true,
		TickIntervalSeconds: 60,
		WindowMinutes:       5,
		TargetTPM:           100000,
		TargetRPM:           60,
		TargetCostRate:      1.0,
		AlphaLoad:           0.3,
		AlphaUp:             0.30,
		AlphaDown:           0.05,
		CostFloorFactor:     1.2,
		MaxFactor:           3.0,
		LoadDeadzone:        0.4,
		HeatGamma:           2.0,
		MaxStepUp:           0.10,
		MaxStepDown:         0.03,
		FailoverProbability: 0.15,
	}
}

const eps = 1e-9

func TestComputeRouteCost(t *testing.T) {
	tests := []struct {
		name   string
		cheap  float64
		backup float64
		p      float64
		want   float64
	}{
		{"normal weighted average", 1.0, 1.8, 0.15, 1.12},
		{"backup unknown falls back to cheap", 1.0, 0.0, 0.15, 1.0},
		{"backup negative falls back to cheap", 1.0, -2.0, 0.15, 1.0},
		{"backup cheaper than cheap falls back", 1.0, 0.8, 0.15, 1.0},
		{"backup equal to cheap uses cheap", 1.0, 1.0, 0.15, 1.0},
		{"p below zero clamped to 0", 1.0, 2.0, -1.0, 1.0},
		{"p above one clamped to 1", 1.0, 2.0, 3.0, 2.0},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := ComputeRouteCost(tt.cheap, tt.backup, tt.p)
			if !approxEqual(got, tt.want, eps) {
				t.Errorf("ComputeRouteCost(%v, %v, %v) = %v, want %v", tt.cheap, tt.backup, tt.p, got, tt.want)
			}
		})
	}
}

func TestRawLoad(t *testing.T) {
	tests := []struct {
		name          string
		tokens        float64
		requests      float64
		costUSD       float64
		windowMinutes float64
		targetTPM     float64
		targetRPM     float64
		targetCost    float64
		want          float64
	}{
		{"single tpm dimension dominates", 600000, 0, 0, 5, 100000, 0, 0, 1.2},
		{"missing targets skipped", 1000, 1000, 1000, 5, 100, 0, 0, 2.0},
		{"max over all dimensions", 600000, 0, 0.5, 5, 100000, 60, 1.0, 1.2},
		{"request dimension dominates", 600000, 600, 0.5, 5, 100000, 60, 1.0, 2.0},
		{"cost-rate dimension dominates", 600000, 0, 10.0, 5, 100000, 60, 1.0, 2.0},
		{"all targets zero gives no signal", 600000, 600, 10.0, 5, 0, 0, 0, 0.0},
		{"zero window gives no signal", 600000, 0, 0, 0, 100000, 0, 0, 0.0},
		{"negative window gives no signal", 600000, 0, 0, -5, 100000, 0, 0, 0.0},
		{"exactly at target is 1.0", 500000, 0, 0, 5, 100000, 0, 0, 1.0},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := RawLoad(tt.tokens, tt.requests, tt.costUSD, tt.windowMinutes,
				tt.targetTPM, tt.targetRPM, tt.targetCost)
			if !approxEqual(got, tt.want, eps) {
				t.Errorf("RawLoad(...) = %v, want %v", got, tt.want)
			}
		})
	}
}

func TestUpdateLoadEMA(t *testing.T) {
	tests := []struct {
		name  string
		prev  float64
		raw   float64
		alpha float64
		want  float64
	}{
		{"cold start takes raw sample", 0, 1.2, 0.3, 1.2},
		{"normal smoothing", 100, 120, 0.3, 106},
		{"alpha zero keeps prev", 100, 120, 0.0, 100},
		{"alpha one takes raw", 100, 120, 1.0, 120},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := UpdateLoadEMA(tt.prev, tt.raw, tt.alpha)
			if !approxEqual(got, tt.want, eps) {
				t.Errorf("UpdateLoadEMA(%v, %v, %v) = %v, want %v", tt.prev, tt.raw, tt.alpha, got, tt.want)
			}
		})
	}
}

func TestHeat(t *testing.T) {
	tests := []struct {
		name     string
		load     float64
		deadzone float64
		gamma    float64
		want     float64
	}{
		{"below deadzone is zero", 0.2, 0.4, 2.0, 0.0},
		{"exactly at deadzone is zero", 0.4, 0.4, 2.0, 0.0},
		{"load one is one", 1.0, 0.4, 2.0, 1.0},
		{"midpoint curvature", 0.7, 0.4, 2.0, 0.25},
		{"load above one clamps to one", 2.0, 0.4, 2.0, 1.0},
		{"gamma one is linear", 0.7, 0.4, 1.0, 0.5},
		{"negative deadzone defaults to zero", 0.7, -1.0, 2.0, 0.49},
		{"deadzone at/above one defaults to zero", 0.7, 1.0, 2.0, 0.49},
		{"deadzone above one defaults to zero", 0.7, 1.5, 2.0, 0.49},
		{"gamma below one defaults to one", 0.7, 0.4, 0.5, 0.5},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := Heat(tt.load, tt.deadzone, tt.gamma)
			if !approxEqual(got, tt.want, eps) {
				t.Errorf("Heat(%v, %v, %v) = %v, want %v", tt.load, tt.deadzone, tt.gamma, got, tt.want)
			}
		})
	}
}

func TestDynamicMultiplier(t *testing.T) {
	tests := []struct {
		name      string
		cost      float64
		maxFactor float64
		heat      float64
		want      float64
	}{
		{"cost unknown returns base price", 0, 3.0, 1.0, 1.0},
		{"cost negative returns base price", -1.0, 3.0, 1.0, 1.0},
		{"zero heat returns base price", 1.0, 3.0, 0.0, 1.0},
		{"full heat at max factor", 1.0, 3.0, 1.0, 3.0},
		{"half heat is midpoint", 1.0, 3.0, 0.5, 2.0},
		{"max factor below one clamped to one", 1.0, 0.5, 1.0, 1.0},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := DynamicMultiplier(tt.cost, tt.maxFactor, tt.heat)
			if !approxEqual(got, tt.want, eps) {
				t.Errorf("DynamicMultiplier(%v, %v, %v) = %v, want %v", tt.cost, tt.maxFactor, tt.heat, got, tt.want)
			}
		})
	}
}

func TestNextMultiplier(t *testing.T) {
	tests := []struct {
		name      string
		prev      float64
		target    float64
		alphaUp   float64
		alphaDown float64
		want      float64
	}{
		{"cold start returns target", 0, 2.0, 0.3, 0.05, 2.0},
		{"negative prev cold start returns target", -1.0, 2.0, 0.3, 0.05, 2.0},
		{"rising uses alpha up", 1.0, 2.0, 0.3, 0.05, 1.3},
		{"falling uses alpha down", 2.0, 1.0, 0.3, 0.05, 1.95},
		{"already at target stays", 2.0, 2.0, 0.3, 0.05, 2.0},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := NextMultiplier(tt.prev, tt.target, tt.alphaUp, tt.alphaDown)
			if !approxEqual(got, tt.want, eps) {
				t.Errorf("NextMultiplier(%v, %v, %v, %v) = %v, want %v",
					tt.prev, tt.target, tt.alphaUp, tt.alphaDown, got, tt.want)
			}
		})
	}
}

func TestEnforceBounds(t *testing.T) {
	tests := []struct {
		name        string
		mult        float64
		prev        float64
		cost        float64
		floorFactor float64
		maxFactor   float64
		maxStepUp   float64
		maxStepDown float64
		want        float64
	}{
		{"cost unknown returns base price", 2.0, 1.0, 0, 1.2, 3.0, 0.10, 0.03, 1.0},
		{"step-up clamp", 1.5, 1.0, 1.0, 1.2, 3.0, 0.10, 0.03, 1.1},
		{"step-up clamp with maxFactor below step bound", 3.0, 1.4, 1.0, 1.2, 1.5, 0.10, 0.03, 1.5},
		{"step-down clamp", 1.0, 2.0, 1.0, 1.2, 3.0, 0.10, 0.03, 1.94},
		{"absolute ceiling", 10.0, 0.0, 1.0, 1.2, 3.0, 0.10, 0.03, 3.0},
		{"absolute floor", 0.5, 0.0, 1.0, 1.2, 3.0, 0.10, 0.03, 1.0},
		{"prev non-positive skips step clamp", 1.5, 0.0, 1.0, 1.2, 3.0, 0.10, 0.03, 1.5},
		{"prev negative skips step clamp", 1.5, -1.0, 1.0, 1.2, 3.0, 0.10, 0.03, 1.5},
		{"max factor below one clamps to one", 0.5, 0.0, 1.0, 1.2, 0.5, 0.10, 0.03, 1.0},
		{"within bounds unchanged", 1.5, 1.4, 1.0, 1.2, 3.0, 0.10, 0.03, 1.5},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := EnforceBounds(tt.mult, tt.prev, tt.cost, tt.floorFactor,
				tt.maxFactor, tt.maxStepUp, tt.maxStepDown)
			if !approxEqual(got, tt.want, eps) {
				t.Errorf("EnforceBounds(%v, %v, %v, %v, %v, %v, %v) = %v, want %v",
					tt.mult, tt.prev, tt.cost, tt.floorFactor, tt.maxFactor,
					tt.maxStepUp, tt.maxStepDown, got, tt.want)
			}
		})
	}
}

func TestEffectiveCost(t *testing.T) {
	tests := []struct {
		name      string
		routeCost float64
		costEMA   float64
		want      float64
	}{
		{"picks route cost when above EMA", 1.15, 0.5, 1.15},
		{"picks EMA when above route cost", 1.15, 2.0, 2.0},
		{"equal values", 1.15, 1.15, 1.15},
		{"zero EMA falls back to route cost", 1.15, 0, 1.15},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := EffectiveCost(tt.routeCost, tt.costEMA, 1.2)
			if !approxEqual(got, tt.want, eps) {
				t.Errorf("EffectiveCost(%v, %v) = %v, want %v", tt.routeCost, tt.costEMA, got, tt.want)
			}
		})
	}
}

// fullLoadInput returns a TickInput that saturates the default TPM target
// (load >= 1) with a known upstream unit cost.
func fullLoadInput(costPerM float64) TickInput {
	return TickInput{
		Model:                 "test-model",
		WindowTokens:          600000,
		WindowRequests:        0,
		WindowUpstreamCostUSD: costPerM * 600000 / 1e6,
		WindowMinutes:         5,
		CheapCost:             costPerM,
		BackupCost:            costPerM * 1.8,
		Now:                   1700000000,
	}
}

// zeroLoadInput returns a TickInput that produces zero load (no tokens) but
// still carries a valid upstream cost so the factor may move.
func zeroLoadInput() TickInput {
	return TickInput{
		Model:         "test-model",
		WindowTokens:  0,
		WindowMinutes: 5,
		CheapCost:     1.0,
		BackupCost:    1.8,
		Now:           1700000000,
	}
}

func TestTickZeroLoadKeepsFactorAtOne(t *testing.T) {
	s := defaultSetting()
	state := &ModelState{Factor: 1.0, UpdatedAt: 0}

	got := Tick(state, zeroLoadInput(), s)

	if !approxEqual(got, 1.0, eps) {
		t.Errorf("Tick(zero load) = %v, want 1.0", got)
	}
	if !approxEqual(state.Factor, 1.0, eps) {
		t.Errorf("state.Factor = %v, want 1.0", state.Factor)
	}
}

func TestTickFullLoadConvergesToMaxFactor(t *testing.T) {
	s := defaultSetting()
	state := &ModelState{Factor: 1.0, UpdatedAt: 0}
	in := fullLoadInput(1.0)

	final := 0.0
	for i := 0; i < 50; i++ {
		final = Tick(state, in, s)
		if final > s.MaxFactor+eps {
			t.Fatalf("tick %d exceeded maxFactor: %v > %v", i, final, s.MaxFactor)
		}
	}

	if final <= 2.5 {
		t.Errorf("factor after 50 full-load ticks = %v, want > 2.5", final)
	}
	if !approxEqual(state.Factor, final, eps) {
		t.Errorf("state.Factor = %v, return value = %v, want equal", state.Factor, final)
	}
}

func TestTickNoCostDataKeepsFactorAtOne(t *testing.T) {
	s := defaultSetting()
	state := &ModelState{Factor: 2.0, UpdatedAt: 0}
	in := TickInput{
		Model:         "test-model",
		WindowTokens:  600000,
		WindowMinutes: 5,
		CheapCost:     0, // feature off: no upstream cost known
		BackupCost:    0,
		Now:           1700000000,
	}

	got := Tick(state, in, s)

	if !approxEqual(got, 1.0, eps) {
		t.Errorf("Tick(no cost data) = %v, want 1.0", got)
	}
	if !approxEqual(state.Factor, 1.0, eps) {
		t.Errorf("state.Factor = %v, want 1.0", state.Factor)
	}
}

// TestTickColdStartFactorZeroIsStepClamped guards the cold-start path: an
// empty state (&ModelState{} with Factor=0, as produced before the service
// layer seeds Factor=1.0) must NOT jump straight to the target multiplier on
// the first tick. Tick treats Factor<=0 as 1.0 so the MaxStepUp clamp binds
// from tick one.
func TestTickColdStartFactorZeroIsStepClamped(t *testing.T) {
	s := defaultSetting()
	state := &ModelState{}

	got := Tick(state, fullLoadInput(1.0), s)

	want := 1.0 * (1 + s.MaxStepUp) // 1.10
	if got > want+eps {
		t.Errorf("Tick(cold start, full load) = %v, want <= %v (step-up clamp must bind on the first tick, not jump to maxFactor %v)", got, want, s.MaxFactor)
	}
	if !approxEqual(got, want, eps) {
		t.Errorf("Tick(cold start, full load) = %v, want %v", got, want)
	}
	if !approxEqual(state.Factor, want, eps) {
		t.Errorf("state.Factor = %v, want %v", state.Factor, want)
	}
}

// TestTickNoCostSignalResetsWarmState covers the no-cost-signal tick: when
// CheapCost, BackupCost and WindowUpstreamCostUSD are all <= 0 (e.g. the
// admin removed the channel_costs config mid-run), a warm CostEMA must not
// keep the multiplier elevated. Tick resets the state to base price and
// returns 1.0 immediately.
func TestTickNoCostSignalResetsWarmState(t *testing.T) {
	s := defaultSetting()
	state := &ModelState{CostEMA: 2.0, LoadEMA: 1.5, Factor: 2.5, UpdatedAt: 0}
	in := TickInput{
		Model:         "test-model",
		WindowTokens:  600000, // token volume alone is not a cost signal
		WindowMinutes: 5,
		CheapCost:     0, // no configured channel cost this tick
		BackupCost:    0,
		Now:           1700000000,
	}

	got := Tick(state, in, s)

	if !approxEqual(got, 1.0, eps) {
		t.Errorf("Tick(no cost signal) = %v, want 1.0", got)
	}
	if !approxEqual(state.Factor, 1.0, eps) {
		t.Errorf("state.Factor = %v, want 1.0", state.Factor)
	}
	if !approxEqual(state.CostEMA, 0, eps) {
		t.Errorf("state.CostEMA = %v, want 0 (stale EMA must not keep the factor elevated)", state.CostEMA)
	}
	if state.UpdatedAt != in.Now {
		t.Errorf("state.UpdatedAt = %v, want %v", state.UpdatedAt, in.Now)
	}
}

func TestTickRisingFasterThanFalling(t *testing.T) {
	s := defaultSetting()

	// One tick at full load from factor 1.0.
	up := &ModelState{Factor: 1.0, UpdatedAt: 0}
	upFactor := Tick(up, fullLoadInput(1.0), s)
	upDelta := upFactor - 1.0

	// One tick at zero load from factor 2.0.
	down := &ModelState{Factor: 2.0, UpdatedAt: 0}
	downFactor := Tick(down, zeroLoadInput(), s)
	downDelta := 2.0 - downFactor

	t.Logf("up delta = %v, down delta = %v", upDelta, downDelta)
	if upDelta <= downDelta {
		t.Errorf("rising delta %v should exceed falling delta %v", upDelta, downDelta)
	}
	if !approxEqual(upFactor, 1.1, eps) {
		t.Errorf("rising tick factor = %v, want 1.1 (step-up clamp)", upFactor)
	}
	if !approxEqual(downFactor, 1.95, eps) {
		t.Errorf("falling tick factor = %v, want 1.95 (alphaDown EMA)", downFactor)
	}
}

func TestTickCostFloorNeverExceedsMaxFactor(t *testing.T) {
	s := defaultSetting()
	state := &ModelState{Factor: 1.0, UpdatedAt: 0}

	// Window cost matches the route cost (cheap=1, backup=2, p=0.15 -> 1.15)
	// so C is pinned to the actual upstream cost.
	in := TickInput{
		Model:                 "test-model",
		WindowTokens:          1e6,
		WindowRequests:        0,
		WindowUpstreamCostUSD: 1.15,
		WindowMinutes:         5,
		CheapCost:             1.0,
		BackupCost:            2.0,
		Now:                   1700000000,
	}

	final := 0.0
	for i := 0; i < 100; i++ {
		final = Tick(state, in, s)
		if final > s.MaxFactor+eps {
			t.Fatalf("tick %d exceeded maxFactor: %v > %v", i, final, s.MaxFactor)
		}
	}
	if !approxEqual(final, s.MaxFactor, eps) {
		t.Errorf("final factor = %v, want %v at full load", final, s.MaxFactor)
	}
}

func TestTickPerStepRiseLimit(t *testing.T) {
	// AlphaUp=1.0 makes the EMA jump straight to the target, so the only
	// thing capping the single tick is the MaxStepUp clamp.
	s := defaultSetting()
	s.AlphaUp = 1.0

	state := &ModelState{Factor: 1.0, UpdatedAt: 0}
	got := Tick(state, fullLoadInput(1.0), s)

	want := 1.0 * (1 + s.MaxStepUp) // 1.10
	if got > want+eps {
		t.Errorf("Tick(alphaUp=1, full load) = %v, want <= %v (per-step rise limit)", got, want)
	}
	if !approxEqual(got, want, eps) {
		t.Errorf("Tick(alphaUp=1, full load) = %v, want %v", got, want)
	}
}

func TestTickStateMutation(t *testing.T) {
	s := defaultSetting()
	state := &ModelState{Factor: 1.0, UpdatedAt: 0}
	in := fullLoadInput(1.0)

	Tick(state, in, s)

	if state.UpdatedAt != in.Now {
		t.Errorf("state.UpdatedAt = %v, want %v", state.UpdatedAt, in.Now)
	}
	// Cold start: LoadEMA takes the raw sample (tpm ratio 600000/5/100000).
	if !approxEqual(state.LoadEMA, 1.2, eps) {
		t.Errorf("state.LoadEMA = %v, want 1.2", state.LoadEMA)
	}
	// Cold start: CostEMA takes the measured unit cost (1.0 USD/1M).
	if !approxEqual(state.CostEMA, 1.0, eps) {
		t.Errorf("state.CostEMA = %v, want 1.0", state.CostEMA)
	}
	if !approxEqual(state.Factor, 1.1, eps) {
		t.Errorf("state.Factor = %v, want 1.1", state.Factor)
	}
}

func TestComputeRouteCostAndTickCombined(t *testing.T) {
	s := defaultSetting()

	routeCost := ComputeRouteCost(1.0, 2.0, 0.15)
	if !approxEqual(routeCost, 1.15, eps) {
		t.Fatalf("ComputeRouteCost(1, 2, 0.15) = %v, want 1.15", routeCost)
	}
	// EffectiveCost picks the larger anchor in both directions (pure fn).
	if got := EffectiveCost(routeCost, 0.5, s.CostFloorFactor); !approxEqual(got, 1.15, eps) {
		t.Errorf("EffectiveCost(1.15, 0.5) = %v, want 1.15 (route cost above EMA)", got)
	}
	if got := EffectiveCost(routeCost, 2.5, s.CostFloorFactor); !approxEqual(got, 2.5, eps) {
		t.Errorf("EffectiveCost(1.15, 2.5) = %v, want 2.5 (EMA above route cost)", got)
	}

	// NOTE: Tick does not write C back into CostEMA; it smooths CostEMA from
	// the measured upstream unit cost (WindowUpstreamCostUSD*1e6/WindowTokens)
	// and only uses max(routeCost, costEMA) internally as the anchor C. So the
	// combined test asserts the measured-cost EMA path that Tick actually
	// performs, while the max semantics are covered by EffectiveCost above.
	in := TickInput{
		Model:                 "test-model",
		WindowTokens:          600000,
		WindowRequests:        0,
		WindowUpstreamCostUSD: 0.69, // measured unit cost == 1.15 USD/1M
		WindowMinutes:         5,
		CheapCost:             1.0,
		BackupCost:            2.0,
		Now:                   1700000000,
	}

	// Warm EMA below the measured cost: CostEMA rises toward 1.15.
	state := &ModelState{Factor: 1.0, CostEMA: 0.5, UpdatedAt: 0}
	Tick(state, in, s)
	want := UpdateLoadEMA(0.5, 1.15, s.AlphaLoad)
	if !approxEqual(state.CostEMA, want, eps) {
		t.Errorf("CostEMA = %v, want %v (smoothed from measured unit cost)", state.CostEMA, want)
	}

	// Warm EMA above the measured cost: CostEMA decays toward 1.15.
	state2 := &ModelState{Factor: 1.0, CostEMA: 2.5, UpdatedAt: 0}
	Tick(state2, in, s)
	want2 := UpdateLoadEMA(2.5, 1.15, s.AlphaLoad)
	if !approxEqual(state2.CostEMA, want2, eps) {
		t.Errorf("CostEMA = %v, want %v (smoothed from measured unit cost)", state2.CostEMA, want2)
	}
}

func TestTickNilState(t *testing.T) {
	if got := Tick(nil, fullLoadInput(1.0), defaultSetting()); !approxEqual(got, 1.0, eps) {
		t.Errorf("Tick(nil, ...) = %v, want 1.0", got)
	}
}
