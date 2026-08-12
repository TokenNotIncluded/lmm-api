package operation_setting

import (
	"testing"

	"github.com/stretchr/testify/require"
)

func TestViolationFeePolicyPrefersExplicitGroupAndContinuesSequence(t *testing.T) {
	original := violationFeeSettings
	t.Cleanup(func() { violationFeeSettings = original })
	violationFeeSettings = ViolationFeeSettings{
		Enabled: true,
		Policies: []ViolationFeePolicy{
			{Groups: []string{"*"}, Enabled: true, AmountsUSD: []float64{0.5}, InitialAmountUSD: 0.5, Multiplier: 2, MaxAmountUSD: 128, PeriodSeconds: 60},
			{Groups: []string{"vip"}, Enabled: true, AmountsUSD: []float64{4}, InitialAmountUSD: 4, Multiplier: 2, MaxAmountUSD: 128, PeriodSeconds: 60},
		},
	}
	policy, ok := ResolveViolationFeePolicy("vip")
	require.True(t, ok)
	require.Equal(t, "vip", policy.Groups[0])
	require.Equal(t, 4.0, policy.AmountForOccurrence(1))
	require.Equal(t, 8.0, policy.AmountForOccurrence(2))
	global, ok := ResolveViolationFeePolicy("default")
	require.True(t, ok)
	require.Equal(t, 0.5, global.AmountForOccurrence(1))
}

func TestValidateViolationFeeSettingsJSONRejectsUnsafeValues(t *testing.T) {
	require.Error(t, ValidateViolationFeeSettingsJSON(`{"enabled":true,"policies":[{"groups":["default"],"period_seconds":0}]}`))
	require.Error(t, ValidateViolationFeeSettingsJSON(`{"enabled":true,"policies":[{"groups":["default"],"amounts_usd":[1000001],"period_seconds":60}]}`))
	require.NoError(t, ValidateViolationFeeSettingsJSON(`{"enabled":true,"policies":[{"groups":["*"],"amounts_usd":[0.5,1,2],"initial_amount_usd":0.5,"multiplier":2,"max_amount_usd":1024,"period_seconds":86400,"drain_balance_when_short":true}]}`))
}
