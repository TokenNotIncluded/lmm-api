package model

import (
	"encoding/json"
	"errors"
	"regexp"
	"strings"
	"time"
	"unicode"
	"unicode/utf8"

	"github.com/QuantumNous/new-api/common"
	"gorm.io/gorm"
	"gorm.io/gorm/clause"
)

const (
	AssistantProfileUnknown      = "unknown"
	AssistantProfileTechnical    = "technical_cost_sensitive"
	AssistantProfileGuided       = "guided_buyer"
	AssistantProfilePromotion    = "promotion_seeker"
	AssistantProfileSecurityRisk = "security_risk"
	AssistantProfileOperator     = "production_operator"
	AssistantProfilePrivacy      = "privacy_conscious"
	AssistantProfileAccessible   = "mobile_accessibility"
	AssistantProfileNormal       = "normal_user"
	AssistantProfileSupport      = "support_seeking"
	AssistantProfileL0Applicant  = "l0_applicant"
	AssistantProfileCustom       = "custom"

	AssistantUserProfileMaxStrategyRunes = 4000
	AssistantUserProfileMaxTags          = 12
	AssistantUserProfileMaxTagRunes      = 48
)

var (
	ErrAssistantProfileKey          = errors.New("assistant profile key is invalid")
	ErrAssistantProfileStrategyLong = errors.New("assistant profile strategy is too long")
	ErrAssistantProfileTagsInvalid  = errors.New("assistant profile tags are invalid")
	assistantProfileSecretPattern   = regexp.MustCompile(`(?i)(password|passwd|api[ _-]?key|access[ _-]?token|refresh[ _-]?token|client[ _-]?secret|secret|credential|密码|密钥|令牌)\s*[:=：]\s*[^\s,;]+`)
)

// AssistantUserProfile is the administrator-maintained profile override for
// one user. It intentionally stores no raw conversations, inferred identity
// traits, or credentials. The assistant may use it internally to choose a
// response strategy, but it is never exposed by a user-facing endpoint.
type AssistantUserProfile struct {
	Id         int    `json:"-" gorm:"primaryKey"`
	UserId     int    `json:"-" gorm:"not null;uniqueIndex"`
	ProfileKey string `json:"-" gorm:"type:varchar(64);not null;default:''"`
	TagsJSON   string `json:"-" gorm:"type:text;not null;default:'[]'"`
	Strategy   string `json:"-" gorm:"type:text;not null;default:''"`
	Enabled    bool   `json:"-" gorm:"not null;default:false"`
	UpdatedBy  int    `json:"-" gorm:"not null;default:0;index"`
	CreatedAt  int64  `json:"-" gorm:"not null"`
	UpdatedAt  int64  `json:"-" gorm:"not null;index"`
}

// AssistantUserProfileView is the administrator-only representation. Keep
// this separate from the database model so a future user-facing serializer
// cannot accidentally expose UpdatedBy, the raw JSON column, or internal
// storage details.
type AssistantUserProfileView struct {
	ProfileKey string   `json:"profile_key"`
	Tags       []string `json:"tags"`
	Strategy   string   `json:"strategy"`
	Enabled    bool     `json:"enabled"`
	UpdatedAt  int64    `json:"updated_at"`
}

func (AssistantUserProfile) TableName() string { return "assistant_user_profiles" }

func validAssistantProfileKeys() map[string]struct{} {
	return map[string]struct{}{
		AssistantProfileUnknown:      {},
		AssistantProfileTechnical:    {},
		AssistantProfileGuided:       {},
		AssistantProfilePromotion:    {},
		AssistantProfileSecurityRisk: {},
		AssistantProfileOperator:     {},
		AssistantProfilePrivacy:      {},
		AssistantProfileAccessible:   {},
		AssistantProfileNormal:       {},
		AssistantProfileSupport:      {},
		AssistantProfileL0Applicant:  {},
		AssistantProfileCustom:       {},
	}
}

// NormalizeAssistantProfileKey is shared by the admin API and the assistant
// runtime so a manually edited value cannot become an arbitrary prompt field.
func NormalizeAssistantProfileKey(value string) (string, error) {
	value = strings.ToLower(strings.TrimSpace(value))
	if value == "" {
		return "", nil
	}
	if _, ok := validAssistantProfileKeys()[value]; !ok {
		return "", ErrAssistantProfileKey
	}
	return value, nil
}

// NormalizeAssistantProfileStrategy removes controls and credential-shaped
// values before a trusted administrator note can reach an assistant model.
// This is a safety boundary, not a substitute for the normal secret filter.
func NormalizeAssistantProfileStrategy(value string) (string, error) {
	value = strings.Map(func(r rune) rune {
		if unicode.IsControl(r) || unicode.In(r, unicode.Cf) {
			return -1
		}
		return r
	}, strings.TrimSpace(value))
	value = assistantProfileSecretPattern.ReplaceAllString(value, "$1: [REDACTED]")
	value = strings.Join(strings.Fields(value), " ")
	if utf8.RuneCountInString(value) > AssistantUserProfileMaxStrategyRunes {
		return "", ErrAssistantProfileStrategyLong
	}
	return value, nil
}

func NormalizeAssistantProfileTags(tags []string) ([]string, error) {
	if len(tags) > AssistantUserProfileMaxTags {
		return nil, ErrAssistantProfileTagsInvalid
	}
	seen := make(map[string]struct{}, len(tags))
	result := make([]string, 0, len(tags))
	for _, tag := range tags {
		tag = strings.Map(func(r rune) rune {
			if unicode.IsControl(r) || unicode.In(r, unicode.Cf) {
				return -1
			}
			return r
		}, strings.TrimSpace(tag))
		tag = strings.Join(strings.Fields(tag), " ")
		if tag == "" {
			continue
		}
		if utf8.RuneCountInString(tag) > AssistantUserProfileMaxTagRunes {
			return nil, ErrAssistantProfileTagsInvalid
		}
		if _, ok := seen[tag]; ok {
			continue
		}
		seen[tag] = struct{}{}
		result = append(result, tag)
	}
	return result, nil
}

func AssistantUserProfileTags(profile *AssistantUserProfile) []string {
	if profile == nil || strings.TrimSpace(profile.TagsJSON) == "" {
		return nil
	}
	var tags []string
	if json.Unmarshal([]byte(profile.TagsJSON), &tags) != nil {
		return nil
	}
	validated, err := NormalizeAssistantProfileTags(tags)
	if err != nil {
		return nil
	}
	return validated
}

func AssistantUserProfileViewOf(profile *AssistantUserProfile) AssistantUserProfileView {
	if profile == nil {
		return AssistantUserProfileView{Tags: []string{}}
	}
	strategy, err := NormalizeAssistantProfileStrategy(profile.Strategy)
	if err != nil {
		strategy = assistantProfileSecretPattern.ReplaceAllString(profile.Strategy, "$1: [REDACTED]")
		strategyRunes := []rune(strategy)
		if len(strategyRunes) > AssistantUserProfileMaxStrategyRunes {
			strategy = string(strategyRunes[:AssistantUserProfileMaxStrategyRunes])
		}
	}
	return AssistantUserProfileView{
		ProfileKey: profile.ProfileKey,
		Tags:       AssistantUserProfileTags(profile),
		Strategy:   strategy,
		Enabled:    profile.Enabled,
		UpdatedAt:  profile.UpdatedAt,
	}
}

func GetAssistantUserProfile(userID int) (*AssistantUserProfile, error) {
	if userID <= 0 {
		return nil, gorm.ErrInvalidData
	}
	var profile AssistantUserProfile
	err := DB.Where("user_id = ?", userID).First(&profile).Error
	if errors.Is(err, gorm.ErrRecordNotFound) {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	return &profile, nil
}

func UpsertAssistantUserProfile(userID, updatedBy int, profileKey string, tags []string, strategy string, enabled bool) (*AssistantUserProfile, error) {
	if userID <= 0 || updatedBy <= 0 {
		return nil, gorm.ErrInvalidData
	}
	profileKey, err := NormalizeAssistantProfileKey(profileKey)
	if err != nil {
		return nil, err
	}
	strategy, err = NormalizeAssistantProfileStrategy(strategy)
	if err != nil {
		return nil, err
	}
	if profileKey == "" {
		enabled = false
	}
	tags, err = NormalizeAssistantProfileTags(tags)
	if err != nil {
		return nil, err
	}
	tagsJSON, err := json.Marshal(tags)
	if err != nil {
		return nil, err
	}
	now := common.GetTimestamp()
	row := &AssistantUserProfile{
		UserId:     userID,
		ProfileKey: profileKey,
		TagsJSON:   string(tagsJSON),
		Strategy:   strategy,
		Enabled:    enabled,
		UpdatedBy:  updatedBy,
		CreatedAt:  now,
		UpdatedAt:  now,
	}
	err = DB.Clauses(clause.OnConflict{
		Columns: []clause.Column{{Name: "user_id"}},
		DoUpdates: clause.AssignmentColumns([]string{
			"profile_key", "tags_json", "strategy", "enabled", "updated_by", "updated_at",
		}),
	}).Create(row).Error
	if err != nil {
		return nil, err
	}
	return GetAssistantUserProfile(userID)
}

// AssistantUserProfileDefault returns an API-safe empty state for users who
// have never received a manual override.
func AssistantUserProfileDefault(userID int) *AssistantUserProfile {
	return &AssistantUserProfile{UserId: userID, Enabled: false}
}

func assistantUserProfileNow() int64 { return time.Now().Unix() }
