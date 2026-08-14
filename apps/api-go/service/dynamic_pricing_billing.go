package service

import (
	"math"

	"github.com/QuantumNous/new-api/common"
	"github.com/QuantumNous/new-api/pkg/dynamic_pricing"
	relaycommon "github.com/QuantumNous/new-api/relay/common"
)

const dynamicPricingRatioKey = "dynamic_pricing"

// dynamicPricingMultiplier returns the request's captured multiplier when the
// pricing helper already attached one. Falling back to the live state keeps
// realtime/audio paths safe even when they did not pass through that helper.
func dynamicPricingMultiplier(info *relaycommon.RelayInfo) float64 {
	if info == nil {
		return 1.0
	}
	if ratios := info.PriceData.OtherRatios(); ratios != nil {
		if multiplier, ok := ratios[dynamicPricingRatioKey]; ok && multiplier > 0 &&
			!math.IsNaN(multiplier) && !math.IsInf(multiplier, 0) {
			return multiplier
		}
	}
	if info.OriginModelName == "" {
		return 1.0
	}
	return dynamic_pricing.GetMultiplier(info.OriginModelName)
}

// applyDynamicPricingToQuota applies the captured dynamic multiplier to a
// quota calculated by a path that historically did not use PriceData's
// OtherRatios (audio, realtime, and tiered settlement). It returns the quota
// saturation marker so callers can preserve the existing audit behavior.
func applyDynamicPricingToQuota(info *relaycommon.RelayInfo, quota int) (int, *common.QuotaClamp) {
	multiplier := dynamicPricingMultiplier(info)
	if multiplier == 1.0 {
		return quota, nil
	}
	return common.QuotaFromFloatChecked(float64(quota) * multiplier)
}
