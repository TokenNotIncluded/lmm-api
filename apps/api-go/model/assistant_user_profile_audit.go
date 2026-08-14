package model

import (
	"crypto/sha256"
	"encoding/hex"
	"sort"
	"strings"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"gorm.io/gorm"
)

// AssistantUserProfileAudit is a privacy-minimized history of automatic
// profile changes. It deliberately contains no conversation, strategy, or
// raw tag values. The tag hashes are only useful for detecting a change; they
// are not intended to reconstruct the user's labels.
type AssistantUserProfileAudit struct {
	Id            int64  `json:"id" gorm:"primaryKey;index:idx_assistant_profile_audit_retention,priority:3"`
	UserId        int    `json:"user_id" gorm:"not null;index:idx_assistant_profile_audit_user_time,priority:1"`
	RequestId     string `json:"request_id" gorm:"type:varchar(64);not null;index"`
	OldProfileKey string `json:"old_profile_key" gorm:"type:varchar(64);not null;default:''"`
	NewProfileKey string `json:"new_profile_key" gorm:"type:varchar(64);not null;default:''"`
	OldTagCount   int    `json:"old_tag_count" gorm:"not null;default:0"`
	NewTagCount   int    `json:"new_tag_count" gorm:"not null;default:0"`
	OldTagsHash   string `json:"old_tags_hash" gorm:"type:char(64);not null;default:''"`
	NewTagsHash   string `json:"new_tags_hash" gorm:"type:char(64);not null;default:''"`
	Source        string `json:"source" gorm:"type:varchar(24);not null;default:assistant;index:idx_assistant_profile_audit_retention,priority:1"`
	CreatedAt     int64  `json:"created_at" gorm:"not null;index:idx_assistant_profile_audit_user_time,priority:2;index:idx_assistant_profile_audit_retention,priority:2"`
}

func (AssistantUserProfileAudit) TableName() string { return "assistant_user_profile_audits" }

// AssistantUserProfileTagsHash returns a stable, one-way fingerprint of the
// normalized tag set. Sorting makes the audit stable when only tag order
// changes; the domain prefix prevents cross-purpose hash reuse.
func AssistantUserProfileTagsHash(tags []string) string {
	normalized, err := NormalizeAssistantProfileTags(tags)
	if err != nil || len(normalized) == 0 {
		return ""
	}
	sort.Strings(normalized)
	hash := sha256.Sum256([]byte("lmm.assistant.profile.tags.v1\x00" + strings.Join(normalized, "\x00")))
	return hex.EncodeToString(hash[:])
}

// RecordAssistantUserProfileAudit appends only the bounded metadata needed
// to explain an automatic profile update. It is intentionally best-effort at
// the caller: a logging database outage must not make chat unavailable.
func RecordAssistantUserProfileAudit(userID int, oldProfile, newProfile *AssistantUserProfile, requestID string) error {
	if userID <= 0 || newProfile == nil {
		return gorm.ErrInvalidData
	}
	oldTags := AssistantUserProfileTags(oldProfile)
	newTags := AssistantUserProfileTags(newProfile)
	requestID = strings.TrimSpace(requestID)
	if requestID == "" {
		requestID = common.NewRequestId()
	}
	if len(requestID) > 64 {
		requestID = requestID[:64]
	}
	record := AssistantUserProfileAudit{
		UserId:        userID,
		RequestId:     requestID,
		OldProfileKey: profileKeyOrEmpty(oldProfile),
		NewProfileKey: newProfile.ProfileKey,
		OldTagCount:   len(oldTags),
		NewTagCount:   len(newTags),
		OldTagsHash:   AssistantUserProfileTagsHash(oldTags),
		NewTagsHash:   AssistantUserProfileTagsHash(newTags),
		Source:        newProfile.Source,
		CreatedAt:     common.GetTimestamp(),
	}
	return DB.Create(&record).Error
}

func profileKeyOrEmpty(profile *AssistantUserProfile) string {
	if profile == nil {
		return ""
	}
	return profile.ProfileKey
}
