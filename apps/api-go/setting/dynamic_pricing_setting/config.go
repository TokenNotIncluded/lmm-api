// Package dynamic_pricing_setting holds the layered configuration for the
// dynamic pricing feature.
//
// Dynamic pricing continuously adjusts the effective price factor of a
// channel based on recent load (TPM/RPM), upstream cost, and the rate at
// which the factor can move. It uses a deadzone around the load targets so
// small fluctuations do not churn the factor, an EMA over load samples to
// smooth noise, and a cost floor so prices never drop below the upstream
// cost scaled by CostFloorFactor. Heat (excess load above the targets) is
// shaped by HeatGamma to make the response non-linear. FailoverProbability
// is the chance of routing a request to another channel when the current
// one is overloaded.
//
// All fields are managed by config.GlobalConfig under the module name
// "dynamic_pricing_setting", so DB keys are "dynamic_pricing_setting.xxx"
// (e.g. dynamic_pricing_setting.enabled, dynamic_pricing_setting.target_tpm).
package dynamic_pricing_setting

import (
	"strconv"

	"github.com/QuantumNous/new-api/setting/config"
	"github.com/samber/lo"
)

// ModelPricingOverride is an optional per-model override of the pricing
// targets. A zero field means "inherit the global target"; an override with
// all fields zero is equivalent to no override at all.
type ModelPricingOverride struct {
	TargetTPM      float64 `json:"target_tpm"`
	TargetRPM      float64 `json:"target_rpm"`
	TargetCostRate float64 `json:"target_cost_rate"`
}

// DynamicPricingSetting is managed by config.GlobalConfig.Register.
// DB keys: dynamic_pricing_setting.<json tag of each field>
type DynamicPricingSetting struct {
	Enabled             bool                            `json:"enabled"`
	TickIntervalSeconds int                             `json:"tick_interval_seconds"`
	WindowMinutes       int                             `json:"window_minutes"`
	TargetTPM           float64                         `json:"target_tpm"`
	TargetRPM           float64                         `json:"target_rpm"`
	TargetCostRate      float64                         `json:"target_cost_rate"`
	AlphaLoad           float64                         `json:"alpha_load"`
	AlphaUp             float64                         `json:"alpha_up"`
	AlphaDown           float64                         `json:"alpha_down"`
	CostFloorFactor     float64                         `json:"cost_floor_factor"`
	MaxFactor           float64                         `json:"max_factor"`
	LoadDeadzone        float64                         `json:"load_deadzone"`
	HeatGamma           float64                         `json:"heat_gamma"`
	MaxStepUp           float64                         `json:"max_step_up"`
	MaxStepDown         float64                         `json:"max_step_down"`
	FailoverProbability float64                         `json:"failover_probability"`
	ChannelCosts        map[string]float64              `json:"channel_costs"`
	PerModel            map[string]ModelPricingOverride `json:"per_model"`
}

var dynamicPricingSetting = DynamicPricingSetting{
	Enabled:             false,
	TickIntervalSeconds: 60,
	WindowMinutes:       5,
	TargetTPM:           100000,
	TargetRPM:           60,
	TargetCostRate:      1.0,
	AlphaLoad:           0.3,
	AlphaUp:             0.30,
	AlphaDown:           0.05,
	CostFloorFactor:     1.2,
	MaxFactor:           3.0,
	LoadDeadzone:        0.4,
	HeatGamma:           2.0,
	MaxStepUp:           0.10,
	MaxStepDown:         0.03,
	FailoverProbability: 0.15,
	ChannelCosts:        make(map[string]float64),
	PerModel:            make(map[string]ModelPricingOverride),
}

func init() {
	config.GlobalConfig.Register("dynamic_pricing_setting", &dynamicPricingSetting)
}

// IsEnabled reports whether the dynamic pricing master switch is on. It is
// the cheap hot-path accessor: it reads only the bool field and performs no
// map copies (GetSetting copies ChannelCosts and PerModel on every call), so
// per-request callers such as the multiplier lookup on the billing path can
// gate on it without allocating.
func IsEnabled() bool {
	return dynamicPricingSetting.Enabled
}

// GetSetting returns a deep copy of the current dynamic pricing setting so
// callers cannot mutate the shared config.
func GetSetting() DynamicPricingSetting {
	return DynamicPricingSetting{
		Enabled:             dynamicPricingSetting.Enabled,
		TickIntervalSeconds: dynamicPricingSetting.TickIntervalSeconds,
		WindowMinutes:       dynamicPricingSetting.WindowMinutes,
		TargetTPM:           dynamicPricingSetting.TargetTPM,
		TargetRPM:           dynamicPricingSetting.TargetRPM,
		TargetCostRate:      dynamicPricingSetting.TargetCostRate,
		AlphaLoad:           dynamicPricingSetting.AlphaLoad,
		AlphaUp:             dynamicPricingSetting.AlphaUp,
		AlphaDown:           dynamicPricingSetting.AlphaDown,
		CostFloorFactor:     dynamicPricingSetting.CostFloorFactor,
		MaxFactor:           dynamicPricingSetting.MaxFactor,
		LoadDeadzone:        dynamicPricingSetting.LoadDeadzone,
		HeatGamma:           dynamicPricingSetting.HeatGamma,
		MaxStepUp:           dynamicPricingSetting.MaxStepUp,
		MaxStepDown:         dynamicPricingSetting.MaxStepDown,
		FailoverProbability: dynamicPricingSetting.FailoverProbability,
		ChannelCosts:        lo.Assign(dynamicPricingSetting.ChannelCosts),
		PerModel:            lo.Assign(dynamicPricingSetting.PerModel),
	}
}

// GetChannelCost returns the upstream cost (USD per 1M tokens) configured for
// the given channel, and whether an entry exists for it.
func GetChannelCost(channelId int) (float64, bool) {
	cost, ok := dynamicPricingSetting.ChannelCosts[strconv.Itoa(channelId)]
	return cost, ok
}

// GetModelTargets returns the effective pricing targets for a model. Per-model
// overrides take precedence field by field; a zero override field (or a model
// with no override at all) inherits the global target.
func GetModelTargets(model string) (tpm, rpm, costRate float64) {
	tpm = dynamicPricingSetting.TargetTPM
	rpm = dynamicPricingSetting.TargetRPM
	costRate = dynamicPricingSetting.TargetCostRate
	if override, ok := dynamicPricingSetting.PerModel[model]; ok {
		if override.TargetTPM > 0 {
			tpm = override.TargetTPM
		}
		if override.TargetRPM > 0 {
			rpm = override.TargetRPM
		}
		if override.TargetCostRate > 0 {
			costRate = override.TargetCostRate
		}
	}
	return tpm, rpm, costRate
}
