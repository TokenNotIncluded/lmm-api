package operation_setting

import (
	"math"

	"github.com/LIghtJUNction/api.lmm.best/setting/config"
)

const checkinTrustLevelCount = 5

// DefaultCheckinLevelMultipliers maps trust levels 0..4 to the fraction of
// the configured base reward range that a user may receive.  Keep this as a
// value (rather than a shared mutable slice) so callers cannot change the
// process-wide defaults accidentally.
var DefaultCheckinLevelMultipliers = [...]float64{0.5, 0.65, 0.8, 0.9, 1}

// CheckinSetting 签到功能配置
type CheckinSetting struct {
	Enabled          bool      `json:"enabled"`           // 是否启用签到功能
	MinQuota         int       `json:"min_quota"`         // 签到基础最小额度奖励
	MaxQuota         int       `json:"max_quota"`         // 签到基础最大额度奖励
	LevelMultipliers []float64 `json:"level_multipliers"` // 信任等级 0..4 的奖励倍率
}

// 默认配置
var checkinSetting = CheckinSetting{
	Enabled:          false, // 默认关闭
	MinQuota:         1000,  // 默认基础最小额度 1000 (约 0.002 USD)
	MaxQuota:         10000, // 默认基础最大额度 10000 (约 0.02 USD)
	LevelMultipliers: append([]float64(nil), DefaultCheckinLevelMultipliers[:]...),
}

func init() {
	// 注册到全局配置管理器
	config.GlobalConfig.Register("checkin_setting", &checkinSetting)
}

// GetCheckinSetting 获取签到配置
func GetCheckinSetting() *CheckinSetting {
	return &checkinSetting
}

// IsCheckinEnabled 是否启用签到功能
func IsCheckinEnabled() bool {
	return checkinSetting.Enabled
}

// GetCheckinQuotaRange 获取签到额度范围
func GetCheckinQuotaRange() (min, max int) {
	return checkinSetting.MinQuota, checkinSetting.MaxQuota
}

// GetCheckinLevelMultipliers returns a normalized copy of the configured
// multipliers. Missing or invalid entries use the safe per-level defaults;
// finite zero values remain valid and can intentionally disable rewards for a
// level. A copy prevents callers from mutating the live global setting.
func GetCheckinLevelMultipliers() []float64 {
	return normalizedCheckinLevelMultipliers(checkinSetting.LevelMultipliers)
}

// GetCheckinQuotaRangeForLevel scales the base range for a trust level. Admin
// and root users are clamped to level 4, while malformed negative levels are
// treated as the lowest user level.
func GetCheckinQuotaRangeForLevel(level int) (min, max int, multiplier float64) {
	multiplier = checkinLevelMultiplier(level, checkinSetting.LevelMultipliers)
	min, max = scaledCheckinQuotaRange(checkinSetting.MinQuota, checkinSetting.MaxQuota, multiplier)
	return min, max, multiplier
}

func normalizedCheckinLevelMultipliers(values []float64) []float64 {
	result := make([]float64, checkinTrustLevelCount)
	for index := range result {
		result[index] = DefaultCheckinLevelMultipliers[index]
		if index < len(values) && validCheckinMultiplier(values[index]) {
			result[index] = values[index]
		}
	}
	return result
}

func validCheckinMultiplier(value float64) bool {
	return !math.IsNaN(value) && !math.IsInf(value, 0) && value >= 0
}

func checkinLevelMultiplier(level int, values []float64) float64 {
	if level < 0 {
		level = 0
	}
	if level >= checkinTrustLevelCount {
		level = checkinTrustLevelCount - 1
	}
	return normalizedCheckinLevelMultipliers(values)[level]
}

func scaledCheckinQuotaRange(baseMin, baseMax int, multiplier float64) (min, max int) {
	if baseMin < 0 {
		baseMin = 0
	}
	if baseMax < baseMin {
		baseMax = baseMin
	}
	if !validCheckinMultiplier(multiplier) {
		multiplier = DefaultCheckinLevelMultipliers[0]
	}
	min = int(math.Round(float64(baseMin) * multiplier))
	max = int(math.Round(float64(baseMax) * multiplier))
	if min < 0 {
		min = 0
	}
	if max < min {
		max = min
	}
	return min, max
}
