package model

import (
	"testing"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/LIghtJUNction/api.lmm.best/setting"
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

func TestValidateOptionValueRejectsUnavailableAssistantReviewModel(t *testing.T) {
	db := setupConsoleActivationTestDB(t)
	require.NoError(t, db.AutoMigrate(&Ability{}))
	require.NoError(t, db.Create(&Ability{Group: "default", Model: "review-live", Enabled: true, ChannelId: 1}).Error)

	require.NoError(t, validateOptionValue("AssistantReviewModel", "review-live"))
	require.Error(t, validateOptionValue("AssistantReviewModel", "review-missing"))
}

func TestValidateIPAccessRoutingOptions(t *testing.T) {
	require.NoError(t, validateOptionValue(setting.IPAccessRoutingRulesOptionKey, setting.DefaultIPAccessRoutingRules))
	require.NoError(t, validateOptionValue(setting.IPAccessRoutingRulesOptionKey, "domain(example.com) -> direct"))
	assert.Error(t, validateOptionValue(setting.IPAccessRoutingRulesOptionKey, "dip(geoip:china) -> reject"))
	for key := range retiredIPAccessOptionKeys {
		assert.ErrorContains(t, validateOptionValue(key, "false"), "retired")
	}
}

func TestUpdateOptionMapRejectsMalformedIPAccessRoutingWithoutMutation(t *testing.T) {
	previousOptions := common.OptionMap
	previousRules := setting.GetIPAccessRoutingRules()
	common.OptionMap = map[string]string{}
	require.NoError(t, setting.UpdateIPAccessRoutingRules(setting.DefaultIPAccessRoutingRules))
	t.Cleanup(func() {
		common.OptionMap = previousOptions
		require.NoError(t, setting.UpdateIPAccessRoutingRules(previousRules))
	})

	assert.Error(t, updateOptionMap(setting.IPAccessRoutingRulesOptionKey, "not a route"))
	assert.Equal(t, setting.DefaultIPAccessRoutingRules, setting.GetIPAccessRoutingRules())
	assert.NotContains(t, common.OptionMap, setting.IPAccessRoutingRulesOptionKey)
}

func TestUpdateOptionMapIgnoresRetiredIPAccessOptions(t *testing.T) {
	previousOptions := common.OptionMap
	common.OptionMap = map[string]string{
		"GlobalIPWhitelistEnabled":  "true",
		"GlobalIPWhitelistCIDRs":    `["203.0.113.8"]`,
		"RegionAccessPolicyEnabled": "true",
		"RegionBlockedCountryCodes": "CN",
	}
	t.Cleanup(func() { common.OptionMap = previousOptions })

	for key := range retiredIPAccessOptionKeys {
		require.NoError(t, updateOptionMap(key, "legacy"))
		assert.NotContains(t, common.OptionMap, key)
	}
}
