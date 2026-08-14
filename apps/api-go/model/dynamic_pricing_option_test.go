package model

import (
	"testing"

	"github.com/QuantumNous/new-api/common"
	"github.com/QuantumNous/new-api/setting/config"
	"github.com/QuantumNous/new-api/setting/dynamic_pricing_setting"

	"github.com/glebarez/sqlite"
	"github.com/stretchr/testify/require"
	"gorm.io/gorm"
)

func setupDynamicPricingOptionTest(t *testing.T) (*gorm.DB, *dynamic_pricing_setting.DynamicPricingSetting) {
	t.Helper()
	previousDB := DB
	previousOptions := common.OptionMap
	db, err := gorm.Open(sqlite.Open(":memory:"), &gorm.Config{})
	require.NoError(t, err)
	require.NoError(t, db.AutoMigrate(&Option{}))
	DB = db
	common.OptionMap = map[string]string{}

	cfg := config.GlobalConfig.Get("dynamic_pricing_setting").(*dynamic_pricing_setting.DynamicPricingSetting)
	previousSetting := dynamic_pricing_setting.GetSetting()
	cfg.Enabled = false
	cfg.MinFactor = 1
	cfg.RequireChannelCost = true
	cfg.BasePriceUSDPerMillion = 1
	cfg.MaxFactor = 3
	cfg.ChannelCosts = map[string]float64{}

	t.Cleanup(func() {
		*cfg = previousSetting
		DB = previousDB
		common.OptionMap = previousOptions
	})
	return db, cfg
}

func TestUpdateOptionsBulkValidatesDynamicPricingAsOneConfiguration(t *testing.T) {
	db, cfg := setupDynamicPricingOptionTest(t)
	values := map[string]string{
		"dynamic_pricing_setting.enabled":       "true",
		"dynamic_pricing_setting.min_factor":    "1.25",
		"dynamic_pricing_setting.channel_costs": `{"7":2.5}`,
	}

	require.NoError(t, UpdateOptionsBulk(values))
	require.True(t, cfg.Enabled)
	require.Equal(t, 1.25, cfg.MinFactor)
	require.Equal(t, 2.5, cfg.ChannelCosts["7"])
	for key, value := range values {
		require.Equal(t, value, requireOptionValue(t, db, key))
	}
}

func TestUpdateOptionsBulkRejectsUnsafeDynamicPricingWithoutWrites(t *testing.T) {
	db, cfg := setupDynamicPricingOptionTest(t)
	err := UpdateOptionsBulk(map[string]string{
		"dynamic_pricing_setting.enabled":    "true",
		"dynamic_pricing_setting.min_factor": "4",
	})
	require.Error(t, err)
	require.False(t, cfg.Enabled)
	require.Equal(t, int64(0), func() int64 {
		var count int64
		require.NoError(t, db.Model(&Option{}).Count(&count).Error)
		return count
	}())
}
