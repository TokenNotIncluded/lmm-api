package service

import (
	"testing"

	"github.com/QuantumNous/new-api/pkg/dynamic_pricing"
	relaycommon "github.com/QuantumNous/new-api/relay/common"
	"github.com/QuantumNous/new-api/setting/config"
	"github.com/QuantumNous/new-api/setting/dynamic_pricing_setting"
	hosttypes "github.com/QuantumNous/new-api/types"

	"github.com/stretchr/testify/require"
)

func setupRequestCostFloorTest(t *testing.T) {
	t.Helper()
	cfg := config.GlobalConfig.Get("dynamic_pricing_setting").(*dynamic_pricing_setting.DynamicPricingSetting)
	previous := dynamic_pricing_setting.GetSetting()
	cfg.Enabled = true
	cfg.MinFactor = 1
	cfg.RequireChannelCost = true
	cfg.BasePriceUSDPerMillion = 1
	cfg.CostFloorFactor = 1.2
	cfg.MaxFactor = 3
	cfg.ChannelCosts = map[string]float64{"7": 5}
	t.Cleanup(func() { *cfg = previous })
}

func TestPrepareDynamicPricingForSelectedChannelRaisesAndReserves(t *testing.T) {
	setupRequestCostFloorTest(t)
	dynamic_pricing.SetState("retry-cost-model", &dynamic_pricing.ModelState{Factor: 1.5})
	billing := &recordingBillingSettler{preConsumedQuota: 100}
	info := &relaycommon.RelayInfo{
		OriginModelName:       "retry-cost-model",
		Billing:               billing,
		FinalPreConsumedQuota: 100,
		PriceData:             hosttypes.PriceData{QuotaToPreConsume: 100},
	}
	info.PriceData.AddOtherRatio(dynamicPricingRatioKey, 1.5)

	require.NoError(t, PrepareDynamicPricingForSelectedChannel(info, 7))
	require.InDelta(t, 6, info.PriceData.OtherRatios()[dynamicPricingRatioKey], 1e-9)
	require.Equal(t, 400, info.PriceData.QuotaToPreConsume)
	require.Equal(t, []int{400}, billing.reserveTargets)
	require.Equal(t, 400, info.FinalPreConsumedQuota)
}

func TestPrepareDynamicPricingForSelectedChannelRaisesPerCallReservation(t *testing.T) {
	setupRequestCostFloorTest(t)
	dynamic_pricing.SetState("retry-task-model", &dynamic_pricing.ModelState{Factor: 1.5})
	billing := &recordingBillingSettler{preConsumedQuota: 100}
	info := &relaycommon.RelayInfo{
		OriginModelName:       "retry-task-model",
		Billing:               billing,
		FinalPreConsumedQuota: 100,
		PriceData:             hosttypes.PriceData{Quota: 100},
	}
	info.PriceData.AddOtherRatio(dynamicPricingRatioKey, 1.5)

	require.NoError(t, PrepareDynamicPricingForSelectedChannel(info, 7))
	require.InDelta(t, 6, info.PriceData.OtherRatios()[dynamicPricingRatioKey], 1e-9)
	require.Equal(t, 400, info.PriceData.Quota)
	require.Equal(t, []int{400}, billing.reserveTargets)
	require.Equal(t, 400, info.FinalPreConsumedQuota)
}

func TestPrepareDynamicPricingForSelectedChannelFailsClosed(t *testing.T) {
	setupRequestCostFloorTest(t)
	info := &relaycommon.RelayInfo{OriginModelName: "unknown-cost-model"}
	require.Error(t, PrepareDynamicPricingForSelectedChannel(info, 8))
}
