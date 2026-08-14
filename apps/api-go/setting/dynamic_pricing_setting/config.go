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
	"encoding/json"
	"fmt"
	"math"
	"reflect"
	"strconv"
	"strings"

	"github.com/LIghtJUNction/api.lmm.best/setting/config"
	"github.com/samber/lo"
)

// ModelPricingOverride is an optional per-model override of the pricing
// targets. A zero field means "inherit the global target"; an override with
// all fields zero is equivalent to no override at all.
type ModelPricingOverride struct {
	TargetTPM              float64 `json:"target_tpm"`
	TargetRPM              float64 `json:"target_rpm"`
	TargetCostRate         float64 `json:"target_cost_rate"`
	BasePriceUSDPerMillion float64 `json:"base_price_usd_per_million"`
}

// DynamicPricingSetting is managed by config.GlobalConfig.Register.
// DB keys: dynamic_pricing_setting.<json tag of each field>
type DynamicPricingSetting struct {
	Enabled                bool                            `json:"enabled"`
	MinFactor              float64                         `json:"min_factor"`
	RequireChannelCost     bool                            `json:"require_channel_cost"`
	TickIntervalSeconds    int                             `json:"tick_interval_seconds"`
	WindowMinutes          int                             `json:"window_minutes"`
	TargetTPM              float64                         `json:"target_tpm"`
	TargetRPM              float64                         `json:"target_rpm"`
	TargetCostRate         float64                         `json:"target_cost_rate"`
	BasePriceUSDPerMillion float64                         `json:"base_price_usd_per_million"`
	AlphaLoad              float64                         `json:"alpha_load"`
	AlphaUp                float64                         `json:"alpha_up"`
	AlphaDown              float64                         `json:"alpha_down"`
	CostFloorFactor        float64                         `json:"cost_floor_factor"`
	MaxFactor              float64                         `json:"max_factor"`
	LoadDeadzone           float64                         `json:"load_deadzone"`
	HeatGamma              float64                         `json:"heat_gamma"`
	MaxStepUp              float64                         `json:"max_step_up"`
	MaxStepDown            float64                         `json:"max_step_down"`
	FailoverProbability    float64                         `json:"failover_probability"`
	ChannelCosts           map[string]float64              `json:"channel_costs"`
	PerModel               map[string]ModelPricingOverride `json:"per_model"`
}

var dynamicPricingSetting = DynamicPricingSetting{
	Enabled:                false,
	MinFactor:              1.0,
	RequireChannelCost:     true,
	TickIntervalSeconds:    60,
	WindowMinutes:          5,
	TargetTPM:              100000,
	TargetRPM:              60,
	TargetCostRate:         1.0,
	BasePriceUSDPerMillion: 1.0,
	AlphaLoad:              0.3,
	AlphaUp:                0.30,
	AlphaDown:              0.05,
	CostFloorFactor:        1.2,
	MaxFactor:              3.0,
	LoadDeadzone:           0.4,
	HeatGamma:              2.0,
	MaxStepUp:              0.10,
	MaxStepDown:            0.03,
	FailoverProbability:    0.15,
	ChannelCosts:           make(map[string]float64),
	PerModel:               make(map[string]ModelPricingOverride),
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
		Enabled:                dynamicPricingSetting.Enabled,
		MinFactor:              dynamicPricingSetting.MinFactor,
		RequireChannelCost:     dynamicPricingSetting.RequireChannelCost,
		TickIntervalSeconds:    dynamicPricingSetting.TickIntervalSeconds,
		WindowMinutes:          dynamicPricingSetting.WindowMinutes,
		TargetTPM:              dynamicPricingSetting.TargetTPM,
		TargetRPM:              dynamicPricingSetting.TargetRPM,
		TargetCostRate:         dynamicPricingSetting.TargetCostRate,
		BasePriceUSDPerMillion: dynamicPricingSetting.BasePriceUSDPerMillion,
		AlphaLoad:              dynamicPricingSetting.AlphaLoad,
		AlphaUp:                dynamicPricingSetting.AlphaUp,
		AlphaDown:              dynamicPricingSetting.AlphaDown,
		CostFloorFactor:        dynamicPricingSetting.CostFloorFactor,
		MaxFactor:              dynamicPricingSetting.MaxFactor,
		LoadDeadzone:           dynamicPricingSetting.LoadDeadzone,
		HeatGamma:              dynamicPricingSetting.HeatGamma,
		MaxStepUp:              dynamicPricingSetting.MaxStepUp,
		MaxStepDown:            dynamicPricingSetting.MaxStepDown,
		FailoverProbability:    dynamicPricingSetting.FailoverProbability,
		ChannelCosts:           lo.Assign(dynamicPricingSetting.ChannelCosts),
		PerModel:               lo.Assign(dynamicPricingSetting.PerModel),
	}
}

// Validate checks the live configuration before a ticker iteration uses it.
// Zero target dimensions disable that dimension. The reference base price
// may remain zero only while the feature is disabled; enabling requires a
// positive value. Every other numeric control is required to be finite and
// within its documented range so malformed admin input cannot produce
// NaN/Inf factors.
func (s DynamicPricingSetting) Validate() error {
	if s.MinFactor < 1 || !isFinite(s.MinFactor) {
		return fmt.Errorf("min_factor must be finite and at least 1")
	}
	if s.TickIntervalSeconds <= 0 {
		return fmt.Errorf("tick_interval_seconds must be positive")
	}
	if s.WindowMinutes <= 0 {
		return fmt.Errorf("window_minutes must be positive")
	}
	if err := validateNonNegative("target_tpm", s.TargetTPM); err != nil {
		return err
	}
	if err := validateNonNegative("target_rpm", s.TargetRPM); err != nil {
		return err
	}
	if err := validateNonNegative("target_cost_rate", s.TargetCostRate); err != nil {
		return err
	}
	if s.BasePriceUSDPerMillion < 0 || !isFinite(s.BasePriceUSDPerMillion) {
		return fmt.Errorf("base_price_usd_per_million must be finite and non-negative")
	}
	if s.Enabled && s.BasePriceUSDPerMillion <= 0 {
		return fmt.Errorf("base_price_usd_per_million must be positive while dynamic pricing is enabled")
	}
	if s.Enabled && !s.RequireChannelCost {
		return fmt.Errorf("require_channel_cost must be true while dynamic pricing is enabled")
	}
	if err := validateUnitInterval("alpha_load", s.AlphaLoad); err != nil {
		return err
	}
	if err := validateUnitInterval("alpha_up", s.AlphaUp); err != nil {
		return err
	}
	if err := validateUnitInterval("alpha_down", s.AlphaDown); err != nil {
		return err
	}
	if s.CostFloorFactor < 1 || !isFinite(s.CostFloorFactor) {
		return fmt.Errorf("cost_floor_factor must be finite and at least 1")
	}
	if s.MaxFactor < 1 || !isFinite(s.MaxFactor) {
		return fmt.Errorf("max_factor must be finite and at least 1")
	}
	if s.MinFactor > s.MaxFactor {
		return fmt.Errorf("min_factor must not exceed max_factor")
	}
	if s.LoadDeadzone < 0 || s.LoadDeadzone >= 1 || !isFinite(s.LoadDeadzone) {
		return fmt.Errorf("load_deadzone must be finite and in [0, 1)")
	}
	if s.HeatGamma < 1 || !isFinite(s.HeatGamma) {
		return fmt.Errorf("heat_gamma must be finite and at least 1")
	}
	if err := validateUnitInterval("max_step_up", s.MaxStepUp); err != nil {
		return err
	}
	if err := validateUnitInterval("max_step_down", s.MaxStepDown); err != nil {
		return err
	}
	if err := validateUnitInterval("failover_probability", s.FailoverProbability); err != nil {
		return err
	}
	for channelID, cost := range s.ChannelCosts {
		if cost <= 0 || !isFinite(cost) {
			return fmt.Errorf("channel_costs[%q] must be finite and positive", channelID)
		}
	}
	if s.Enabled && s.RequireChannelCost && len(s.ChannelCosts) == 0 {
		return fmt.Errorf("channel_costs must contain at least one positive channel cost while dynamic pricing is enabled")
	}
	for model, override := range s.PerModel {
		if err := validateNonNegative(fmt.Sprintf("per_model[%q].target_tpm", model), override.TargetTPM); err != nil {
			return err
		}
		if err := validateNonNegative(fmt.Sprintf("per_model[%q].target_rpm", model), override.TargetRPM); err != nil {
			return err
		}
		if err := validateNonNegative(fmt.Sprintf("per_model[%q].target_cost_rate", model), override.TargetCostRate); err != nil {
			return err
		}
		if override.BasePriceUSDPerMillion < 0 || !isFinite(override.BasePriceUSDPerMillion) {
			return fmt.Errorf("per_model[%q].base_price_usd_per_million must be finite and non-negative", model)
		}
	}
	return nil
}

// ValidateOptionValues applies one or more flattened
// dynamic_pricing_setting.<field> values to an isolated copy and validates the
// resulting configuration. It is used by the generic option API and bulk
// updates so malformed values can never be persisted or partially applied.
func ValidateOptionValues(values map[string]string) error {
	candidate := GetSetting()
	for key, value := range values {
		fieldName := strings.TrimPrefix(key, "dynamic_pricing_setting.")
		if fieldName == key {
			continue
		}
		if err := setStrictConfigField(&candidate, fieldName, value); err != nil {
			return err
		}
	}
	return candidate.Validate()
}

// IsOptionKey reports whether key belongs to the dynamic-pricing config.
func IsOptionKey(key string) bool {
	return strings.HasPrefix(key, "dynamic_pricing_setting.")
}

func setStrictConfigField(candidate *DynamicPricingSetting, key, value string) error {
	rv := reflect.ValueOf(candidate).Elem()
	rt := rv.Type()
	for i := 0; i < rv.NumField(); i++ {
		fieldType := rt.Field(i)
		fieldKey := strings.Split(fieldType.Tag.Get("json"), ",")[0]
		if fieldKey == "" {
			fieldKey = fieldType.Name
		}
		if fieldKey != key {
			continue
		}

		field := rv.Field(i)
		if field.Kind() == reflect.String {
			field.SetString(value)
			return nil
		}
		parsed := reflect.New(field.Type())
		if err := json.Unmarshal([]byte(value), parsed.Interface()); err != nil {
			return fmt.Errorf("dynamic_pricing_setting.%s is invalid: %w", key, err)
		}
		field.Set(parsed.Elem())
		return nil
	}
	return fmt.Errorf("unknown dynamic pricing setting %q", key)
}

func isFinite(value float64) bool {
	return !math.IsNaN(value) && !math.IsInf(value, 0)
}

func validateNonNegative(name string, value float64) error {
	if value < 0 || !isFinite(value) {
		return fmt.Errorf("%s must be finite and non-negative", name)
	}
	return nil
}

func validateUnitInterval(name string, value float64) error {
	if value < 0 || value > 1 || !isFinite(value) {
		return fmt.Errorf("%s must be finite and in [0, 1]", name)
	}
	return nil
}

// GetChannelCost returns the upstream cost (USD per 1M tokens) configured for
// the given channel, and whether an entry exists for it.
func GetChannelCost(channelId int) (float64, bool) {
	cost, ok := dynamicPricingSetting.ChannelCosts[strconv.Itoa(channelId)]
	return cost, ok && cost > 0 && isFinite(cost)
}

// GetMinFactor returns the configured request-path floor. Invalid live state
// falls back to 1.0; validated option writes prevent this in normal operation.
func GetMinFactor() float64 {
	if dynamicPricingSetting.MinFactor < 1 || !isFinite(dynamicPricingSetting.MinFactor) {
		return 1.0
	}
	return dynamicPricingSetting.MinFactor
}

// RequiresChannelCost reports whether requests on channels without a
// configured conservative cost must fail closed.
func RequiresChannelCost() bool {
	return dynamicPricingSetting.RequireChannelCost
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

// GetModelBasePrice returns the USD per 1M-token reference price used to turn
// an upstream cost floor into a multiplier. A zero override inherits the
// global value; a zero global value is kept backwards-compatible as 1.0.
func GetModelBasePrice(model string) float64 {
	base := dynamicPricingSetting.BasePriceUSDPerMillion
	if override, ok := dynamicPricingSetting.PerModel[model]; ok && override.BasePriceUSDPerMillion > 0 {
		base = override.BasePriceUSDPerMillion
	}
	if base <= 0 || !isFinite(base) {
		return 1.0
	}
	return base
}

// GetMaxFactor returns a safe request-path ceiling without copying the full
// setting. Invalid live configuration falls back to the neutral ceiling.
func GetMaxFactor() float64 {
	if dynamicPricingSetting.MaxFactor < 1 || !isFinite(dynamicPricingSetting.MaxFactor) {
		return 1.0
	}
	return dynamicPricingSetting.MaxFactor
}
