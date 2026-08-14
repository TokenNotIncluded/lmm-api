// Read-only admin status endpoints for the dynamic pricing feature.
package controller

import (
	"fmt"
	"sort"
	"strconv"
	"strings"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/LIghtJUNction/api.lmm.best/model"
	"github.com/LIghtJUNction/api.lmm.best/pkg/dynamic_pricing"
	"github.com/LIghtJUNction/api.lmm.best/service"
	"github.com/LIghtJUNction/api.lmm.best/setting/dynamic_pricing_setting"

	"github.com/gin-gonic/gin"
)

type dynamicPricingSettingUpdate struct {
	Enabled                *bool               `json:"enabled"`
	MinFactor              *float64            `json:"min_factor"`
	BasePriceUSDPerMillion *float64            `json:"base_price_usd_per_million"`
	CostFloorFactor        *float64            `json:"cost_floor_factor"`
	MaxFactor              *float64            `json:"max_factor"`
	ChannelCosts           *map[string]float64 `json:"channel_costs"`
}

func activeChannelCostCoverage(s dynamic_pricing_setting.DynamicPricingSetting) (active int, configured int, channels []gin.H, missing []gin.H, err error) {
	dbChannels, err := model.GetAllChannels(0, 0, true, false)
	if err != nil {
		return 0, 0, nil, nil, err
	}
	for _, channel := range dbChannels {
		if channel == nil || channel.Status != common.ChannelStatusEnabled {
			continue
		}
		active++
		cost, hasCost := s.ChannelCosts[strconv.Itoa(channel.Id)]
		channels = append(channels, gin.H{
			"id":         channel.Id,
			"name":       channel.Name,
			"cost":       cost,
			"cost_floor": dynamic_pricing.CostFloorMultiplier(cost, s.BasePriceUSDPerMillion, s.CostFloorFactor),
			"configured": hasCost && cost > 0,
		})
		if hasCost && cost > 0 {
			configured++
			continue
		}
		missing = append(missing, gin.H{
			"id":   channel.Id,
			"name": channel.Name,
		})
	}
	sort.Slice(missing, func(i, j int) bool {
		return missing[i]["id"].(int) < missing[j]["id"].(int)
	})
	sort.Slice(channels, func(i, j int) bool {
		return channels[i]["id"].(int) < channels[j]["id"].(int)
	})
	return active, configured, channels, missing, nil
}

func dynamicPricingRequestFactorRange(engineFactor, basePrice, floorFactor float64, channels []gin.H) (float64, float64) {
	minimum := engineFactor
	maximum := engineFactor
	foundConfiguredChannel := false
	for _, channel := range channels {
		cost, ok := channel["cost"].(float64)
		if !ok || cost <= 0 {
			continue
		}
		requestFactor := dynamic_pricing.CostFloorMultiplier(cost, basePrice, floorFactor)
		if requestFactor <= 0 {
			continue
		}
		requestFactor = max(engineFactor, requestFactor)
		if !foundConfiguredChannel {
			minimum = requestFactor
			maximum = requestFactor
			foundConfiguredChannel = true
			continue
		}
		minimum = min(minimum, requestFactor)
		maximum = max(maximum, requestFactor)
	}
	return minimum, maximum
}

// GetDynamicPricingStatus returns a read-only snapshot of the dynamic pricing
// feature for the admin console: whether it is enabled, the full setting
// (none of its fields are sensitive), and the per-model runtime state
// (factor, load/cost EMA, last tick time).
func GetDynamicPricingStatus(c *gin.Context) {
	s := dynamic_pricing_setting.GetSetting()
	configErr := s.Validate()
	activeChannels, configuredChannels, channels, missingChannels, coverageErr := activeChannelCostCoverage(s)

	previewFactor := 1.0
	if s.Enabled {
		previewFactor = s.MinFactor
		_, previewFactor = dynamicPricingRequestFactorRange(
			previewFactor,
			s.BasePriceUSDPerMillion,
			s.CostFloorFactor,
			channels,
		)
	}

	models := make(map[string]gin.H, 0)
	modelNames := dynamic_pricing.AllModels()
	sort.Strings(modelNames)
	for _, modelName := range modelNames {
		state, ok := dynamic_pricing.GetState(modelName)
		if !ok || state == nil {
			continue
		}
		engineFactor := dynamic_pricing.GetMultiplier(modelName)
		requestFactorMin := engineFactor
		requestFactorMax := engineFactor
		if s.Enabled {
			requestFactorMin, requestFactorMax = dynamicPricingRequestFactorRange(
				engineFactor,
				dynamic_pricing_setting.GetModelBasePrice(modelName),
				s.CostFloorFactor,
				channels,
			)
		}
		models[modelName] = gin.H{
			"factor":               engineFactor,
			"request_factor_min":   requestFactorMin,
			"request_factor_max":   requestFactorMax,
			"engine_factor":        state.Factor,
			"hard_cost_floor":      state.CostFloor,
			"load_ema":             state.LoadEMA,
			"cost_ema":             state.CostEMA,
			"has_unpriced_traffic": state.HasUnpricedTraffic,
			"unpriced_tokens":      state.UnpricedTokens,
			"unpriced_requests":    state.UnpricedRequests,
			"updated_at":           state.UpdatedAt,
		}
		if s.Enabled {
			previewFactor = max(previewFactor, requestFactorMax)
		}
	}

	ready := configErr == nil && coverageErr == nil && s.RequireChannelCost && len(missingChannels) == 0
	status := "ready"
	reason := ""
	if configErr != nil {
		status = "invalid_configuration"
		reason = configErr.Error()
	} else if coverageErr != nil {
		status = "coverage_check_failed"
		reason = coverageErr.Error()
	} else if !s.RequireChannelCost {
		status = "cost_guard_disabled"
		reason = "unknown-cost channels are not configured to fail closed"
	} else if len(missingChannels) > 0 {
		status = "missing_channel_costs"
		reason = "one or more active channels do not have a conservative upstream cost"
	}
	common.ApiSuccess(c, gin.H{
		"enabled":        s.Enabled,
		"preview_factor": previewFactor,
		"setting":        s,
		"models":         models,
		"safety": gin.H{
			"ready":                    ready,
			"status":                   status,
			"reason":                   reason,
			"active_channel_count":     activeChannels,
			"configured_channel_count": configuredChannels,
			"channels":                 channels,
			"missing_channels":         missingChannels,
			"require_channel_cost":     s.RequireChannelCost,
		},
	})
}

// UpdateDynamicPricingSetting atomically validates and stores the operator-
// facing controls used by the settings page. The master switch is applied
// only after the cost inputs, so requests never observe an enabled but
// half-configured safety policy.
func UpdateDynamicPricingSetting(c *gin.Context) {
	var request dynamicPricingSettingUpdate
	if err := common.DecodeJson(c.Request.Body, &request); err != nil {
		common.ApiErrorMsg(c, "invalid dynamic pricing settings")
		return
	}

	candidate := dynamic_pricing_setting.GetSetting()
	values := make(map[string]string)
	if request.Enabled != nil {
		candidate.Enabled = *request.Enabled
		values["dynamic_pricing_setting.enabled"] = strconv.FormatBool(*request.Enabled)
		if *request.Enabled {
			// The operator-facing switch always enables the fail-closed cost
			// policy as part of the same atomic update.
			candidate.RequireChannelCost = true
			values["dynamic_pricing_setting.require_channel_cost"] = "true"
		}
	}
	if request.MinFactor != nil {
		candidate.MinFactor = *request.MinFactor
		values["dynamic_pricing_setting.min_factor"] = strconv.FormatFloat(*request.MinFactor, 'f', -1, 64)
	}
	if request.BasePriceUSDPerMillion != nil {
		candidate.BasePriceUSDPerMillion = *request.BasePriceUSDPerMillion
		values["dynamic_pricing_setting.base_price_usd_per_million"] = strconv.FormatFloat(*request.BasePriceUSDPerMillion, 'f', -1, 64)
	}
	if request.CostFloorFactor != nil {
		candidate.CostFloorFactor = *request.CostFloorFactor
		values["dynamic_pricing_setting.cost_floor_factor"] = strconv.FormatFloat(*request.CostFloorFactor, 'f', -1, 64)
	}
	if request.MaxFactor != nil {
		candidate.MaxFactor = *request.MaxFactor
		values["dynamic_pricing_setting.max_factor"] = strconv.FormatFloat(*request.MaxFactor, 'f', -1, 64)
	}
	if request.ChannelCosts != nil {
		candidate.ChannelCosts = *request.ChannelCosts
		encoded, err := common.Marshal(candidate.ChannelCosts)
		if err != nil {
			common.ApiError(c, err)
			return
		}
		values["dynamic_pricing_setting.channel_costs"] = string(encoded)
	}
	if len(values) == 0 {
		common.ApiErrorMsg(c, "no dynamic pricing settings were supplied")
		return
	}
	if err := candidate.Validate(); err != nil {
		common.ApiErrorMsg(c, err.Error())
		return
	}
	if candidate.Enabled && candidate.RequireChannelCost {
		_, _, _, missing, err := activeChannelCostCoverage(candidate)
		if err != nil {
			common.ApiError(c, err)
			return
		}
		if len(missing) > 0 {
			labels := make([]string, 0, len(missing))
			for _, channel := range missing {
				labels = append(labels, fmt.Sprintf("%v (%v)", channel["name"], channel["id"]))
			}
			common.ApiErrorMsg(c, "configure a positive conservative cost for every active channel before enabling dynamic pricing: "+strings.Join(labels, ", "))
			return
		}
	}
	if err := model.UpdateOptionsBulk(values); err != nil {
		common.ApiError(c, err)
		return
	}
	recordManageAudit(c, "dynamic_pricing.update", map[string]interface{}{
		"keys": len(values),
	})
	service.RunDynamicPricingTickNow()
	GetDynamicPricingStatus(c)
}
