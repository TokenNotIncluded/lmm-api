package relay

import (
	"testing"

	"github.com/QuantumNous/new-api/constant"
	relaycommon "github.com/QuantumNous/new-api/relay/common"
	hosttypes "github.com/QuantumNous/new-api/types"

	"github.com/stretchr/testify/require"
)

func TestTaskSubmitQuotaKeepsDynamicPricingForPricePatch(t *testing.T) {
	previous := constant.TaskPricePatches
	constant.TaskPricePatches = []string{"patched-model"}
	t.Cleanup(func() { constant.TaskPricePatches = previous })

	info := &relaycommon.RelayInfo{PriceData: hosttypes.PriceData{Quota: 100}}
	info.PriceData.AddOtherRatio("duration", 3)
	info.PriceData.AddOtherRatio("dynamic_pricing", 2)

	quota, clamp := taskSubmitQuotaWithRatios(info, "patched-model")
	require.Nil(t, clamp)
	require.Equal(t, 200, quota, "price patches skip adaptor dimensions but must retain dynamic pricing")

	quota, clamp = taskSubmitQuotaWithRatios(info, "ordinary-model")
	require.Nil(t, clamp)
	require.Equal(t, 600, quota)
}

func TestMidjourneyPerCallQuotaAppliesDynamicPricing(t *testing.T) {
	priceData := hosttypes.PriceData{Quota: 100}
	priceData.AddOtherRatio("dynamic_pricing", 2.5)

	require.NoError(t, applyMidjourneyPriceRatios(&priceData))
	require.Equal(t, 250, priceData.Quota)
}
