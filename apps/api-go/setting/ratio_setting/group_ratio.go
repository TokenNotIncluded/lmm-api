package ratio_setting

import (
	"encoding/json"
	"errors"
	"fmt"
	"math"
	"strings"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/LIghtJUNction/api.lmm.best/setting/config"
	"github.com/LIghtJUNction/api.lmm.best/types"
)

var defaultGroupRatio = map[string]float64{
	"default": 1,
	"vip":     1,
	"svip":    1,
}

var groupRatioMap = types.NewRWMap[string, float64]()

var defaultGroupGroupRatio = map[string]map[string]float64{
	"vip": {
		"edit_this": 0.9,
	},
}

var groupGroupRatioMap = types.NewRWMap[string, map[string]float64]()

var defaultGroupSpecialUsableGroup = map[string]map[string]string{}

// GroupWarning is the per-routing-group acknowledgement policy. Keeping this
// in the group settings makes the policy data-driven: administrators can
// configure any group without adding another special case to token creation.
type GroupWarning struct {
	Enabled       bool   `json:"enabled"`
	Message       string `json:"message"`
	Mode          string `json:"mode"`
	Confirmations int    `json:"confirmations"`
}

var defaultGroupWarnings = map[string]GroupWarning{}
var groupWarningMap = types.NewRWMap[string, GroupWarning]()

type GroupRatioSetting struct {
	GroupRatio              *types.RWMap[string, float64]            `json:"group_ratio"`
	GroupGroupRatio         *types.RWMap[string, map[string]float64] `json:"group_group_ratio"`
	GroupSpecialUsableGroup *types.RWMap[string, map[string]string]  `json:"group_special_usable_group"`
	GroupWarnings           *types.RWMap[string, GroupWarning]       `json:"group_warnings"`
}

var groupRatioSetting GroupRatioSetting

func init() {
	groupSpecialUsableGroup := types.NewRWMap[string, map[string]string]()
	groupSpecialUsableGroup.AddAll(defaultGroupSpecialUsableGroup)

	groupRatioMap.AddAll(defaultGroupRatio)
	groupGroupRatioMap.AddAll(defaultGroupGroupRatio)
	groupWarningMap.AddAll(defaultGroupWarnings)

	groupRatioSetting = GroupRatioSetting{
		GroupSpecialUsableGroup: groupSpecialUsableGroup,
		GroupRatio:              groupRatioMap,
		GroupGroupRatio:         groupGroupRatioMap,
		GroupWarnings:           groupWarningMap,
	}

	config.GlobalConfig.Register("group_ratio_setting", &groupRatioSetting)
}

func GetGroupRatioSetting() *GroupRatioSetting {
	if groupRatioSetting.GroupSpecialUsableGroup == nil {
		groupRatioSetting.GroupSpecialUsableGroup = types.NewRWMap[string, map[string]string]()
		groupRatioSetting.GroupSpecialUsableGroup.AddAll(defaultGroupSpecialUsableGroup)
	}
	if groupRatioSetting.GroupWarnings == nil {
		groupRatioSetting.GroupWarnings = groupWarningMap
	}
	return &groupRatioSetting
}

func normalizeGroupWarning(warning GroupWarning) (GroupWarning, error) {
	warning.Message = strings.TrimSpace(warning.Message)
	warning.Mode = strings.ToLower(strings.TrimSpace(warning.Mode))
	if warning.Mode == "" {
		warning.Mode = "modal"
	}
	if warning.Mode != "modal" && warning.Mode != "banner" && warning.Mode != "inline" {
		return GroupWarning{}, fmt.Errorf("group warning mode must be modal, banner, or inline")
	}
	if len([]rune(warning.Message)) > 2000 {
		return GroupWarning{}, fmt.Errorf("group warning message must be at most 2000 characters")
	}
	if warning.Enabled && warning.Message == "" {
		return GroupWarning{}, fmt.Errorf("enabled group warnings require a message")
	}
	if warning.Confirmations == 0 {
		warning.Confirmations = 1
	}
	if warning.Confirmations < 1 || warning.Confirmations > 3 {
		return GroupWarning{}, fmt.Errorf("group warning confirmations must be between 1 and 3")
	}
	return warning, nil
}

func GetGroupWarningsCopy() map[string]GroupWarning {
	return groupWarningMap.ReadAll()
}

func GetGroupWarning(group string) (GroupWarning, bool) {
	group = strings.TrimSpace(group)
	if group == "" {
		return GroupWarning{}, false
	}
	if warning, ok := groupWarningMap.Get(group); ok {
		if warning.Enabled {
			return warning, true
		}
		return GroupWarning{}, false
	}
	for configuredGroup, warning := range groupWarningMap.ReadAll() {
		if strings.EqualFold(strings.TrimSpace(configuredGroup), group) {
			if warning.Enabled {
				return warning, true
			}
			// An explicit disabled entry is an administrator decision and must
			// suppress the data-driven zero-ratio fallback below.
			return GroupWarning{}, false
		}
	}
	// A zero-ratio group is the data-driven default for a public/free relay.
	// This keeps the safety policy useful on a fresh install without baking a
	// particular group identifier into the application.
	if GetGroupRatio(group) == 0 {
		return GroupWarning{
			Enabled:       true,
			Message:       "This routing group is community-operated. Availability, model coverage, privacy handling, and billing behavior may be less predictable. Do not send secrets or sensitive data. Continue only if you accept these risks.",
			Mode:          "modal",
			Confirmations: 3,
		}, true
	}
	return GroupWarning{}, false
}

func GroupWarnings2JSONString() string {
	return groupWarningMap.MarshalJSONString()
}

func UpdateGroupWarningsByJSONString(jsonStr string) error {
	var raw map[string]GroupWarning
	if err := json.Unmarshal([]byte(jsonStr), &raw); err != nil {
		return err
	}
	if raw == nil {
		return errors.New("group warnings must be a JSON object")
	}
	normalized := make(map[string]GroupWarning, len(raw))
	for group, warning := range raw {
		trimmedGroup := strings.TrimSpace(group)
		if err := validateGroupKey(trimmedGroup, "group warning"); err != nil {
			return err
		}
		clean, err := normalizeGroupWarning(warning)
		if err != nil {
			return fmt.Errorf("group %q: %w", trimmedGroup, err)
		}
		normalized[trimmedGroup] = clean
	}
	return types.LoadFromJsonString(groupWarningMap, string(mustMarshalGroupWarnings(normalized)))
}

func mustMarshalGroupWarnings(warnings map[string]GroupWarning) []byte {
	bytes, err := json.Marshal(warnings)
	if err != nil {
		return []byte("{}")
	}
	return bytes
}

func CheckGroupWarnings(jsonStr string) error {
	var raw map[string]GroupWarning
	if err := json.Unmarshal([]byte(jsonStr), &raw); err != nil {
		return err
	}
	if raw == nil {
		return errors.New("group warnings must be a JSON object")
	}
	for group, warning := range raw {
		if err := validateGroupKey(group, "group warning"); err != nil {
			return err
		}
		if _, err := normalizeGroupWarning(warning); err != nil {
			return fmt.Errorf("group %q: %w", group, err)
		}
	}
	return nil
}

func GetGroupRatioCopy() map[string]float64 {
	return groupRatioMap.ReadAll()
}

func ContainsGroupRatio(name string) bool {
	_, ok := groupRatioMap.Get(name)
	return ok
}

func GroupRatio2JSONString() string {
	return groupRatioMap.MarshalJSONString()
}

func UpdateGroupRatioByJSONString(jsonStr string) error {
	return types.LoadFromJsonString(groupRatioMap, jsonStr)
}

func GetGroupRatio(name string) float64 {
	ratio, ok := groupRatioMap.Get(name)
	if !ok {
		common.SysLog("group ratio not found: " + name)
		return 1
	}
	return ratio
}

func GetGroupGroupRatio(userGroup, usingGroup string) (float64, bool) {
	gp, ok := groupGroupRatioMap.Get(userGroup)
	if !ok {
		return -1, false
	}
	ratio, ok := gp[usingGroup]
	if !ok {
		return -1, false
	}
	return ratio, true
}

func GroupGroupRatio2JSONString() string {
	return groupGroupRatioMap.MarshalJSONString()
}

func UpdateGroupGroupRatioByJSONString(jsonStr string) error {
	parsed, err := parseGroupGroupRatio(jsonStr)
	if err != nil {
		return err
	}
	// Parse and validate before replacing the live map. RWMap's generic JSON
	// loader clears the map before decoding, which would otherwise turn a
	// malformed admin edit into an empty runtime policy.
	groupGroupRatioMap.Clear()
	groupGroupRatioMap.AddAll(parsed)
	return nil
}

func validateGroupKey(group, kind string) error {
	trimmed := strings.TrimSpace(group)
	if trimmed == "" {
		return fmt.Errorf("%s keys must not be empty", kind)
	}
	if len([]rune(trimmed)) > 64 {
		return fmt.Errorf("%s keys must be at most 64 characters", kind)
	}
	return nil
}

func parseGroupGroupRatio(jsonStr string) (map[string]map[string]float64, error) {
	var raw map[string]map[string]float64
	if err := json.Unmarshal([]byte(jsonStr), &raw); err != nil {
		return nil, err
	}
	if raw == nil {
		return nil, errors.New("group group ratio must be a JSON object")
	}
	for userGroup, ratios := range raw {
		if err := validateGroupKey(userGroup, "group group ratio"); err != nil {
			return nil, err
		}
		if ratios == nil {
			return nil, fmt.Errorf("group group ratio %q must contain a JSON object", userGroup)
		}
		for usingGroup, ratio := range ratios {
			if err := validateGroupKey(usingGroup, "group group ratio"); err != nil {
				return nil, err
			}
			if ratio < 0 || math.IsNaN(ratio) || math.IsInf(ratio, 0) {
				return nil, fmt.Errorf("group group ratio %q[%q] must be finite and non-negative", userGroup, usingGroup)
			}
		}
	}
	return raw, nil
}

// CheckGroupGroupRatio validates the nested per-user-group multiplier map
// without changing the live configuration.
func CheckGroupGroupRatio(jsonStr string) error {
	_, err := parseGroupGroupRatio(jsonStr)
	return err
}

func CheckGroupRatio(jsonStr string) error {
	checkGroupRatio := make(map[string]float64)
	err := json.Unmarshal([]byte(jsonStr), &checkGroupRatio)
	if err != nil {
		return err
	}
	for name, ratio := range checkGroupRatio {
		if ratio < 0 {
			return errors.New("group ratio must be not less than 0: " + name)
		}
	}
	return nil
}
