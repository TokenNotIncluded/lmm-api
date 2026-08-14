package operation_setting

import (
	"encoding/json"
	"fmt"
	"math"
	"strings"

	"github.com/LIghtJUNction/api.lmm.best/setting/config"
)

const (
	ViolationFeeOptionKey        = "violation_fee"
	ViolationFeeMaxPolicies      = 128
	ViolationFeeMaxAmounts       = 64
	ViolationFeeMaxPeriodSeconds = 366 * 24 * 60 * 60
	ViolationFeeMaxAmountUSD     = 1_000_000
)

// ViolationFeePolicy is a global usage-policy penalty selected by the
// user's effective group. It deliberately has no model/provider field.
type ViolationFeePolicy struct {
	Name                  string    `json:"name,omitempty"`
	Groups                []string  `json:"groups"`
	Enabled               bool      `json:"enabled"`
	AmountsUSD            []float64 `json:"amounts_usd,omitempty"`
	InitialAmountUSD      float64   `json:"initial_amount_usd"`
	Multiplier            float64   `json:"multiplier"`
	MaxAmountUSD          float64   `json:"max_amount_usd"`
	PeriodSeconds         int64     `json:"period_seconds"`
	DrainBalanceWhenShort bool      `json:"drain_balance_when_short"`
}

type ViolationFeeSettings struct {
	Enabled  bool                 `json:"enabled"`
	Policies []ViolationFeePolicy `json:"policies"`
}

var defaultViolationFeeSettings = ViolationFeeSettings{
	Enabled: true,
	Policies: []ViolationFeePolicy{{
		Name:                  "global-default",
		Groups:                []string{"*"},
		Enabled:               true,
		AmountsUSD:            []float64{0.5, 1, 2, 4, 8, 16, 32, 64},
		InitialAmountUSD:      0.5,
		Multiplier:            2,
		MaxAmountUSD:          ViolationFeeMaxAmountUSD,
		PeriodSeconds:         30 * 24 * 60 * 60,
		DrainBalanceWhenShort: true,
	}},
}

var violationFeeSettings = defaultViolationFeeSettings

func init() {
	config.GlobalConfig.Register(ViolationFeeOptionKey, &violationFeeSettings)
}

func GetViolationFeeSettings() *ViolationFeeSettings {
	return &violationFeeSettings
}

func cloneViolationFeePolicy(policy ViolationFeePolicy) ViolationFeePolicy {
	policy.Groups = append([]string(nil), policy.Groups...)
	policy.AmountsUSD = append([]float64(nil), policy.AmountsUSD...)
	return policy
}

// ResolveViolationFeePolicy uses the first explicit group match and falls
// back to a policy containing "*". Specific groups therefore take priority
// over the default policy regardless of where the default appears.
func ResolveViolationFeePolicy(group string) (ViolationFeePolicy, bool) {
	group = strings.TrimSpace(group)
	if !violationFeeSettings.Enabled {
		return ViolationFeePolicy{}, false
	}
	for _, policy := range violationFeeSettings.Policies {
		if !policy.Enabled || !policyMatchesGroup(policy, group, false) {
			continue
		}
		return cloneViolationFeePolicy(policy), true
	}
	for _, policy := range violationFeeSettings.Policies {
		if !policy.Enabled || !policyMatchesGroup(policy, group, true) {
			continue
		}
		return cloneViolationFeePolicy(policy), true
	}
	return ViolationFeePolicy{}, false
}

func policyMatchesGroup(policy ViolationFeePolicy, group string, wildcardOnly bool) bool {
	for _, value := range policy.Groups {
		value = strings.TrimSpace(value)
		if wildcardOnly {
			if value == "*" {
				return true
			}
			continue
		}
		if value != "" && value != "*" && value == group {
			return true
		}
	}
	return false
}

func (policy ViolationFeePolicy) Key() string {
	for _, group := range policy.Groups {
		group = strings.TrimSpace(group)
		if group != "" && group != "*" {
			return "group:" + group
		}
	}
	return "global"
}

func (policy ViolationFeePolicy) AmountForOccurrence(occurrence int) float64 {
	if occurrence < 1 {
		occurrence = 1
	}
	var amount float64
	if occurrence <= len(policy.AmountsUSD) {
		amount = policy.AmountsUSD[occurrence-1]
	} else {
		amount = policy.InitialAmountUSD
		if amount <= 0 && len(policy.AmountsUSD) > 0 {
			amount = policy.AmountsUSD[0]
		}
		if amount > 0 {
			for step := 1; step < occurrence; step++ {
				if policy.Multiplier > 1 && amount < policy.MaxAmountUSD {
					amount *= policy.Multiplier
				}
				if amount >= policy.MaxAmountUSD {
					amount = policy.MaxAmountUSD
					break
				}
			}
		}
	}
	if policy.MaxAmountUSD > 0 && amount > policy.MaxAmountUSD {
		amount = policy.MaxAmountUSD
	}
	if amount < 0 || math.IsNaN(amount) || math.IsInf(amount, 0) {
		return 0
	}
	return amount
}

func ValidateViolationFeeSettingsJSON(value string) error {
	var settings ViolationFeeSettings
	if err := json.Unmarshal([]byte(value), &settings); err != nil {
		return fmt.Errorf("violation fee settings must be valid JSON: %w", err)
	}
	if len(settings.Policies) > ViolationFeeMaxPolicies {
		return fmt.Errorf("violation fee policy count cannot exceed %d", ViolationFeeMaxPolicies)
	}
	for index, policy := range settings.Policies {
		if len(policy.Groups) == 0 {
			return fmt.Errorf("violation fee policy %d must specify at least one group", index+1)
		}
		if len(policy.AmountsUSD) > ViolationFeeMaxAmounts {
			return fmt.Errorf("violation fee policy %d has too many amounts", index+1)
		}
		if policy.InitialAmountUSD < 0 || policy.InitialAmountUSD > ViolationFeeMaxAmountUSD ||
			policy.MaxAmountUSD < 0 || policy.MaxAmountUSD > ViolationFeeMaxAmountUSD {
			return fmt.Errorf("violation fee policy %d has an unsafe amount", index+1)
		}
		if policy.Multiplier < 0 || policy.Multiplier > 100 {
			return fmt.Errorf("violation fee policy %d has an unsafe multiplier", index+1)
		}
		if policy.PeriodSeconds <= 0 || policy.PeriodSeconds > ViolationFeeMaxPeriodSeconds {
			return fmt.Errorf("violation fee policy %d has an invalid period", index+1)
		}
		for _, amount := range policy.AmountsUSD {
			if amount < 0 || amount > ViolationFeeMaxAmountUSD || math.IsNaN(amount) || math.IsInf(amount, 0) {
				return fmt.Errorf("violation fee policy %d has an unsafe amount sequence", index+1)
			}
		}
	}
	return nil
}
