package model

import (
	"testing"

	"github.com/QuantumNous/new-api/common"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestValidateOptionValueRejectsInvalidMaxTokenAutoGroups(t *testing.T) {
	for _, value := range []string{"", "0", "-1", "1.5", "invalid"} {
		t.Run(value, func(t *testing.T) {
			assert.Error(t, validateOptionValue("MaxTokenAutoGroups", value))
		})
	}
	require.NoError(t, validateOptionValue("MaxTokenAutoGroups", "999999"))
}

func TestValidateRegionPolicyOptions(t *testing.T) {
	for _, value := range []string{"", "1", "yes", "TRUE", "False"} {
		t.Run("enabled/"+value, func(t *testing.T) {
			assert.Error(t, validateOptionValue(common.RegionAccessPolicyEnabledOptionKey, value))
		})
	}
	for _, value := range []string{"true", "false"} {
		t.Run("enabled/"+value, func(t *testing.T) {
			require.NoError(t, validateOptionValue(common.RegionAccessPolicyEnabledOptionKey, value))
		})
	}

	for _, value := range []string{"", "CN;US", "USA", "C"} {
		t.Run("blocked-countries/"+value, func(t *testing.T) {
			assert.Error(t, validateOptionValue(common.RegionBlockedCountryCodesOptionKey, value))
		})
	}
	require.NoError(t, validateOptionValue(common.RegionBlockedCountryCodesOptionKey, " cn,US,CN "))
}

func TestUpdateOptionMapRejectsMalformedRegionPolicyWithoutMutation(t *testing.T) {
	previousOptions := common.OptionMap
	previousEnabled := common.IsRegionAccessPolicyEnabled()
	previousCodes := common.RegionBlockedCountryCodesString()
	common.OptionMap = map[string]string{}
	common.SetRegionAccessPolicyEnabled(true)
	_ = common.SetRegionBlockedCountryCodes("CN")
	t.Cleanup(func() {
		common.OptionMap = previousOptions
		common.SetRegionAccessPolicyEnabled(previousEnabled)
		_ = common.SetRegionBlockedCountryCodes(previousCodes)
	})

	assert.Error(t, updateOptionMap(common.RegionAccessPolicyEnabledOptionKey, "not-a-bool"))
	assert.True(t, common.IsRegionAccessPolicyEnabled())
	assert.NotContains(t, common.OptionMap, common.RegionAccessPolicyEnabledOptionKey)

	assert.Error(t, updateOptionMap(common.RegionBlockedCountryCodesOptionKey, "CN;US"))
	assert.Equal(t, "CN", common.RegionBlockedCountryCodesString())
	assert.NotContains(t, common.OptionMap, common.RegionBlockedCountryCodesOptionKey)
}
