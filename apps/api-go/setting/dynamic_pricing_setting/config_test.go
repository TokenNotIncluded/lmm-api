package dynamic_pricing_setting

import "testing"

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
