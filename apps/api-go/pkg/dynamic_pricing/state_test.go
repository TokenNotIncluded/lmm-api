package dynamic_pricing

import (
	"sort"
	"testing"

	"github.com/QuantumNous/new-api/common"
	"github.com/QuantumNous/new-api/setting/config"
	"github.com/QuantumNous/new-api/setting/dynamic_pricing_setting"
)

// disableRedis forces the in-memory-only path (no live RDB in tests) and
// restores common.RedisEnabled afterwards, mirroring the pattern used in
// model/user_session_test.go.
func disableRedis(t *testing.T) {
	t.Helper()
	old := common.RedisEnabled
	common.RedisEnabled = false
	t.Cleanup(func() { common.RedisEnabled = old })
}

// resetStatesForTest clears the package-global store so tests are
// independent of each other's SetState calls.
func resetStatesForTest() {
	statesMu.Lock()
	states = map[string]*ModelState{}
	statesMu.Unlock()
}

// setDynamicPricingEnabledForTest mutates the registered dynamic pricing
// setting's Enabled flag (the same pointer config.GlobalConfig holds, mirroring
// service/dynamic_pricing_test.go) and restores the previous setting on
// cleanup, so tests can exercise the GetMultiplier gate.
func setDynamicPricingEnabledForTest(t *testing.T, enabled bool) {
	t.Helper()
	cfg := config.GlobalConfig.Get("dynamic_pricing_setting").(*dynamic_pricing_setting.DynamicPricingSetting)
	oldCfg := dynamic_pricing_setting.GetSetting()
	cfg.Enabled = enabled
	t.Cleanup(func() { *cfg = oldCfg })
}

func TestGetMultiplierDefaultsToOne(t *testing.T) {
	disableRedis(t)
	resetStatesForTest()
	if got := GetMultiplier("no-such-model"); got != 1.0 {
		t.Fatalf("GetMultiplier(missing) = %v, want 1.0", got)
	}
	SetState("zero-factor", &ModelState{Factor: 0})
	if got := GetMultiplier("zero-factor"); got != 1.0 {
		t.Fatalf("GetMultiplier(factor 0) = %v, want 1.0", got)
	}
}

func TestSetGetStateRoundTrip(t *testing.T) {
	disableRedis(t)
	setDynamicPricingEnabledForTest(t, true)
	model := "roundtrip-model"
	SetState(model, &ModelState{LoadEMA: 1.5, CostEMA: 2.0, Factor: 2.5, UpdatedAt: 12345})

	st, ok := GetState(model)
	if !ok {
		t.Fatal("GetState after SetState returned ok=false")
	}
	if st.Factor != 2.5 || st.LoadEMA != 1.5 || st.CostEMA != 2.0 || st.UpdatedAt != 12345 {
		t.Fatalf("GetState = %+v, want LoadEMA=1.5 CostEMA=2.0 Factor=2.5 UpdatedAt=12345", st)
	}
	if got := GetMultiplier(model); got != 2.5 {
		t.Fatalf("GetMultiplier = %v, want 2.5", got)
	}

	// GetState must return a copy: mutating it must not affect the store.
	st.Factor = 99
	if got := GetMultiplier(model); got != 2.5 {
		t.Fatalf("GetMultiplier after mutating returned copy = %v, want 2.5 (copy semantics violated)", got)
	}
}

func TestSetStateReplacesAndDeleteNotSupported(t *testing.T) {
	disableRedis(t)
	setDynamicPricingEnabledForTest(t, true)
	model := "replace-model"
	SetState(model, &ModelState{Factor: 1.5})
	SetState(model, &ModelState{Factor: 3.0})
	if got := GetMultiplier(model); got != 3.0 {
		t.Fatalf("GetMultiplier after replace = %v, want 3.0", got)
	}
}

// TestGetMultiplierRespectsEnabledSetting: while the feature is disabled the
// request path must not charge stale in-memory factors (the ticker has frozen
// them, but GetMultiplier gates on the setting); re-enabling resumes them.
func TestGetMultiplierRespectsEnabledSetting(t *testing.T) {
	disableRedis(t)
	resetStatesForTest()
	setDynamicPricingEnabledForTest(t, true)

	SetState("gate-model", &ModelState{Factor: 2.0})
	if got := GetMultiplier("gate-model"); got != 2.0 {
		t.Fatalf("GetMultiplier(enabled, factor 2.0) = %v, want 2.0", got)
	}

	cfg := config.GlobalConfig.Get("dynamic_pricing_setting").(*dynamic_pricing_setting.DynamicPricingSetting)
	cfg.Enabled = false
	if got := GetMultiplier("gate-model"); got != 1.0 {
		t.Fatalf("GetMultiplier(disabled, stale factor 2.0) = %v, want 1.0", got)
	}

	cfg.Enabled = true
	if got := GetMultiplier("gate-model"); got != 2.0 {
		t.Fatalf("GetMultiplier(re-enabled, factor 2.0) = %v, want 2.0", got)
	}
}

func TestLoadFromRedisUnavailable(t *testing.T) {
	disableRedis(t)
	if st, ok := LoadFromRedis("any-model"); ok || st != nil {
		t.Fatalf("LoadFromRedis with Redis disabled = (%v, %v), want (nil, false)", st, ok)
	}
}

func TestAllModels(t *testing.T) {
	disableRedis(t)
	resetStatesForTest()
	want := []string{"m-a", "m-b", "m-c"}
	for _, m := range want {
		SetState(m, &ModelState{Factor: 1.0})
	}
	got := AllModels()
	sort.Strings(got)
	if len(got) != len(want) {
		t.Fatalf("AllModels = %v, want %v", got, want)
	}
	for i := range want {
		if got[i] != want[i] {
			t.Fatalf("AllModels = %v, want %v", got, want)
		}
	}
}
