package dynamic_pricing_setting

import (
	"math"
	"testing"
)

// saveSettingSnapshot deep-copies the package-global setting (GetSetting
// returns copies of ChannelCosts/PerModel via lo.Assign) and restores the
// whole struct on cleanup, so tests can mutate dynamicPricingSetting freely
// without leaking into other tests.
func saveSettingSnapshot(t *testing.T) {
	t.Helper()
	old := GetSetting()
	t.Cleanup(func() { dynamicPricingSetting = old })
}

func TestGetChannelCost(t *testing.T) {
	saveSettingSnapshot(t)
	dynamicPricingSetting.ChannelCosts = map[string]float64{
		"7": 1.5,
		"0": 0.25, // channel id 0 is a legitimate configured key, not "absent"
	}

	cost, ok := GetChannelCost(7)
	if !ok || cost != 1.5 {
		t.Fatalf("GetChannelCost(7) = (%v, %v), want (1.5, true)", cost, ok)
	}
	cost, ok = GetChannelCost(0)
	if !ok || cost != 0.25 {
		t.Fatalf("GetChannelCost(0) = (%v, %v), want (0.25, true)", cost, ok)
	}
	cost, ok = GetChannelCost(999)
	if ok || cost != 0 {
		t.Fatalf("GetChannelCost(999) = (%v, %v), want (0, false)", cost, ok)
	}
}

func TestGetModelTargets(t *testing.T) {
	saveSettingSnapshot(t)
	dynamicPricingSetting.TargetTPM = 100000
	dynamicPricingSetting.TargetRPM = 60
	dynamicPricingSetting.TargetCostRate = 1.0
	dynamicPricingSetting.PerModel = map[string]ModelPricingOverride{
		"override-tpm":      {TargetTPM: 50000},
		"override-all-zero": {},
	}

	tpm, rpm, costRate := GetModelTargets("no-override")
	if tpm != 100000 || rpm != 60 || costRate != 1.0 {
		t.Fatalf("GetModelTargets(no override) = (%v, %v, %v), want (100000, 60, 1.0)", tpm, rpm, costRate)
	}

	// Override with only TargetTPM set: inherits global RPM and costRate,
	// uses the override TPM.
	tpm, rpm, costRate = GetModelTargets("override-tpm")
	if tpm != 50000 || rpm != 60 || costRate != 1.0 {
		t.Fatalf("GetModelTargets(override tpm only) = (%v, %v, %v), want (50000, 60, 1.0)", tpm, rpm, costRate)
	}

	// Override with all fields zero is equivalent to no override at all.
	tpm, rpm, costRate = GetModelTargets("override-all-zero")
	if tpm != 100000 || rpm != 60 || costRate != 1.0 {
		t.Fatalf("GetModelTargets(all-zero override) = (%v, %v, %v), want (100000, 60, 1.0)", tpm, rpm, costRate)
	}
}

func TestGetModelBasePrice(t *testing.T) {
	saveSettingSnapshot(t)
	dynamicPricingSetting.BasePriceUSDPerMillion = 2.5
	dynamicPricingSetting.PerModel = map[string]ModelPricingOverride{
		"custom": {BasePriceUSDPerMillion: 5},
	}

	if got := GetModelBasePrice("inherit"); got != 2.5 {
		t.Fatalf("GetModelBasePrice(inherit) = %v, want 2.5", got)
	}
	if got := GetModelBasePrice("custom"); got != 5 {
		t.Fatalf("GetModelBasePrice(custom) = %v, want 5", got)
	}
}

func TestValidateRejectsUnsafePricingControls(t *testing.T) {
	valid := GetSetting()
	if err := valid.Validate(); err != nil {
		t.Fatalf("default setting should validate: %v", err)
	}

	tests := []struct {
		name   string
		mutate func(*DynamicPricingSetting)
	}{
		{"max factor NaN", func(s *DynamicPricingSetting) { s.MaxFactor = math.NaN() }},
		{"alpha out of range", func(s *DynamicPricingSetting) { s.AlphaLoad = 1.1 }},
		{"step below zero", func(s *DynamicPricingSetting) { s.MaxStepDown = -0.1 }},
		{"negative channel cost", func(s *DynamicPricingSetting) { s.ChannelCosts = map[string]float64{"1": -1} }},
		{"infinite model base price", func(s *DynamicPricingSetting) {
			s.PerModel = map[string]ModelPricingOverride{"m": {BasePriceUSDPerMillion: math.Inf(1)}}
		}},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			setting := valid
			tt.mutate(&setting)
			if err := setting.Validate(); err == nil {
				t.Fatal("Validate returned nil for unsafe setting")
			}
		})
	}
}
