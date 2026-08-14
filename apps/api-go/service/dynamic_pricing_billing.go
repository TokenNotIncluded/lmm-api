package service

import (
	"fmt"
	"math"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/LIghtJUNction/api.lmm.best/pkg/dynamic_pricing"
	relaycommon "github.com/LIghtJUNction/api.lmm.best/relay/common"
	"github.com/LIghtJUNction/api.lmm.best/setting/dynamic_pricing_setting"
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

// PrepareDynamicPricingForSelectedChannel revalidates the captured request
// multiplier before every upstream attempt. A retry can land on a more
// expensive channel than the initial route; in that case this function raises
// (never lowers) the request ratio and reserves the corresponding larger
// pre-consume amount before any upstream spend.
func PrepareDynamicPricingForSelectedChannel(info *relaycommon.RelayInfo, channelID int) error {
	if info == nil || !dynamic_pricing_setting.IsEnabled() {
		return nil
	}

	multiplier, _, err := dynamic_pricing.GetRequestMultiplier(info.OriginModelName, channelID)
	if err != nil {
		return err
	}

	oldMultiplier := 1.0
	if ratios := info.PriceData.OtherRatios(); ratios != nil {
		if captured, ok := ratios[dynamicPricingRatioKey]; ok && captured > 0 {
			oldMultiplier = captured
		}
	}
	if multiplier <= oldMultiplier {
		return nil
	}

	info.PriceData.AddOtherRatio(dynamicPricingRatioKey, multiplier)
	if info.TieredBillingSnapshot != nil {
		// PrepareTieredBillingForSelectedGroup, called immediately afterwards,
		// recomputes and reserves the tiered estimate with the raised ratio.
		return nil
	}

	quotaToReserve := info.PriceData.QuotaToPreConsume
	usesPerCallQuota := false
	if quotaToReserve <= 0 && info.PriceData.Quota > 0 {
		quotaToReserve = info.PriceData.Quota
		usesPerCallQuota = true
	}
	if quotaToReserve > 0 {
		target, convertErr := common.QuotaFromFloatStrict(math.Ceil(
			float64(quotaToReserve) / oldMultiplier * multiplier,
		))
		if convertErr != nil {
			return fmt.Errorf("dynamic pricing retry reservation is invalid: %w", convertErr)
		}
		if usesPerCallQuota {
			info.PriceData.Quota = target
		} else {
			info.PriceData.QuotaToPreConsume = target
		}
		if info.Billing != nil {
			if reserveErr := info.Billing.Reserve(target); reserveErr != nil {
				return fmt.Errorf("dynamic pricing retry reservation failed: %w", reserveErr)
			}
			info.FinalPreConsumedQuota = info.Billing.GetPreConsumedQuota()
		}
	}
	return nil
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
