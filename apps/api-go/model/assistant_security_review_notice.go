package model

import (
	"encoding/json"
	"errors"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"gorm.io/gorm/clause"
)

const (
	assistantSecurityReviewNoticeKeep = 1
	assistantSecurityReviewNoticeMax  = 100
	assistantSecurityReviewBucketMax  = 20
	assistantSecurityReviewKeyMax     = 128
)

// AssistantSecurityReviewNotice is a bounded, aggregate-only notification
// produced by a successful assistant review. It has no user or request
// identity and is intentionally separate from per-conversation incidents.
type AssistantSecurityReviewNotice struct {
	ID                int64  `json:"id" gorm:"primaryKey"`
	TaskID            string `json:"-" gorm:"type:varchar(64);not null;uniqueIndex"`
	WindowStart       int64  `json:"window_start" gorm:"not null;index"`
	WindowEnd         int64  `json:"window_end" gorm:"not null;index"`
	TotalMatches      int64  `json:"total_matches" gorm:"not null"`
	BlockedMatches    int64  `json:"blocked_matches" gorm:"not null"`
	AuditedMatches    int64  `json:"audited_matches" gorm:"not null"`
	AffectedRequests  int64  `json:"affected_requests" gorm:"not null"`
	AffectedUsers     int64  `json:"affected_users" gorm:"not null"`
	ByCategoryJSON    string `json:"-" gorm:"type:text;not null"`
	ByRuleJSON        string `json:"-" gorm:"type:text;not null"`
	ErrorLogCount     int64  `json:"error_log_count" gorm:"not null;default:0"`
	ErrorChannelsJSON string `json:"-" gorm:"type:text;not null;default:''"`
	ErrorModelsJSON   string `json:"-" gorm:"type:text;not null;default:''"`
	CreatedAt         int64  `json:"created_at" gorm:"not null;index"`
	UpdatedAt         int64  `json:"updated_at" gorm:"not null;index"`
}

func (AssistantSecurityReviewNotice) TableName() string { return "assistant_security_review_notices" }

func SaveAssistantSecurityReviewNotice(taskID string, windowStart, windowEnd int64, review AssistantSecurityReview, now int64) error {
	if DB == nil {
		return errors.New("database is not initialized")
	}
	if taskID == "" || windowStart <= 0 || windowEnd < windowStart {
		return errors.New("assistant security review notice window is invalid")
	}
	if review.TotalMatches <= 0 && review.ErrorLogCount <= 0 {
		return nil
	}
	if now <= 0 {
		now = common.GetTimestamp()
	}
	byCategory, err := json.Marshal(boundSecurityReviewBuckets(review.ByCategory))
	if err != nil {
		return err
	}
	byRule, err := json.Marshal(boundSecurityReviewBuckets(review.ByRule))
	if err != nil {
		return err
	}
	errorChannels, err := json.Marshal(boundSecurityReviewBuckets(review.ErrorChannels))
	if err != nil {
		return err
	}
	errorModels, err := json.Marshal(boundSecurityReviewBuckets(review.ErrorModels))
	if err != nil {
		return err
	}
	notice := AssistantSecurityReviewNotice{
		TaskID:            taskID,
		WindowStart:       windowStart,
		WindowEnd:         windowEnd,
		TotalMatches:      review.TotalMatches,
		BlockedMatches:    review.BlockedMatches,
		AuditedMatches:    review.AuditedMatches,
		AffectedRequests:  review.AffectedRequests,
		AffectedUsers:     review.AffectedUsers,
		ByCategoryJSON:    string(byCategory),
		ByRuleJSON:        string(byRule),
		ErrorLogCount:     review.ErrorLogCount,
		ErrorChannelsJSON: string(errorChannels),
		ErrorModelsJSON:   string(errorModels),
		CreatedAt:         now,
		UpdatedAt:         now,
	}
	return DB.Clauses(clause.OnConflict{Columns: []clause.Column{{Name: "task_id"}}, DoNothing: true}).Create(&notice).Error
}

func (notice AssistantSecurityReviewNotice) Aggregate() (AssistantSecurityReview, error) {
	review := AssistantSecurityReview{
		TotalMatches:     notice.TotalMatches,
		BlockedMatches:   notice.BlockedMatches,
		AuditedMatches:   notice.AuditedMatches,
		AffectedRequests: notice.AffectedRequests,
		AffectedUsers:    notice.AffectedUsers,
		ErrorLogCount:    notice.ErrorLogCount,
	}
	if notice.ByCategoryJSON != "" && json.Unmarshal([]byte(notice.ByCategoryJSON), &review.ByCategory) != nil {
		return AssistantSecurityReview{}, errors.New("assistant security review categories are invalid")
	}
	if notice.ByRuleJSON != "" && json.Unmarshal([]byte(notice.ByRuleJSON), &review.ByRule) != nil {
		return AssistantSecurityReview{}, errors.New("assistant security review rules are invalid")
	}
	if notice.ErrorChannelsJSON != "" && json.Unmarshal([]byte(notice.ErrorChannelsJSON), &review.ErrorChannels) != nil {
		return AssistantSecurityReview{}, errors.New("assistant security review error channels are invalid")
	}
	if notice.ErrorModelsJSON != "" && json.Unmarshal([]byte(notice.ErrorModelsJSON), &review.ErrorModels) != nil {
		return AssistantSecurityReview{}, errors.New("assistant security review error models are invalid")
	}
	review.ByCategory = boundSecurityReviewBuckets(review.ByCategory)
	review.ByRule = boundSecurityReviewBuckets(review.ByRule)
	review.ErrorChannels = boundSecurityReviewBuckets(review.ErrorChannels)
	review.ErrorModels = boundSecurityReviewBuckets(review.ErrorModels)
	return review, nil
}

func boundSecurityReviewBuckets(rows []AdvancedSecurityStatBucket) []AdvancedSecurityStatBucket {
	if len(rows) > assistantSecurityReviewBucketMax {
		rows = rows[:assistantSecurityReviewBucketMax]
	}
	bounded := make([]AdvancedSecurityStatBucket, 0, len(rows))
	for _, row := range rows {
		key := []rune(row.Key)
		if len(key) > assistantSecurityReviewKeyMax {
			key = key[:assistantSecurityReviewKeyMax]
		}
		bounded = append(bounded, AdvancedSecurityStatBucket{Key: string(key), Count: row.Count})
	}
	return bounded
}

func PruneAssistantSecurityReviewNotices(keep int) error {
	if DB == nil {
		return errors.New("database is not initialized")
	}
	if keep <= 0 {
		keep = assistantSecurityReviewNoticeKeep
	}
	if keep > assistantSecurityReviewNoticeMax {
		keep = assistantSecurityReviewNoticeMax
	}
	var ids []int64
	if err := DB.Model(&AssistantSecurityReviewNotice{}).Order("id DESC").Limit(keep).Pluck("id", &ids).Error; err != nil {
		return err
	}
	if len(ids) == 0 {
		return nil
	}
	return DB.Where("id NOT IN ?", ids).Delete(&AssistantSecurityReviewNotice{}).Error
}
