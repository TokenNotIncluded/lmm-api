package model

import (
	"context"
	"errors"
	"strconv"

	"gorm.io/gorm"
)

const assistantErrorReviewBucketLimit = 20

// AssistantErrorLogReview contains only bounded aggregates from error logs.
// It intentionally never selects Content, request IDs, users, IPs, or the
// Other column. Channel IDs and model IDs are enough for an administrator to
// inspect configuration without copying sensitive log payloads into a review.
type AssistantErrorLogReview struct {
	Count    int64                        `json:"count"`
	Channels []AdvancedSecurityStatBucket `json:"channels"`
	Models   []AdvancedSecurityStatBucket `json:"models"`
}

// GetAssistantErrorLogReview summarizes the error log window in the log
// database. The query is deliberately aggregate-only and bounded so the
// scheduled assistant review cannot grow memory with raw log rows.
func GetAssistantErrorLogReview(ctx context.Context, start, end int64) (AssistantErrorLogReview, error) {
	if start <= 0 || end < start {
		return AssistantErrorLogReview{}, errors.New("assistant error review window is invalid")
	}
	if ctx == nil {
		ctx = context.Background()
	}
	if err := ctx.Err(); err != nil {
		return AssistantErrorLogReview{}, err
	}
	if LOG_DB == nil {
		// The log store is an optional deployment component. Keep the main
		// assistant review useful when it is unavailable; production instances
		// with LOG_DB configured still receive the full aggregate inspection.
		return AssistantErrorLogReview{}, nil
	}

	query := LOG_DB.WithContext(ctx).Model(&Log{}).
		Where("type = ? AND created_at >= ? AND created_at <= ?", LogTypeError, start, end)
	var result AssistantErrorLogReview
	if err := query.Count(&result.Count).Error; err != nil {
		return AssistantErrorLogReview{}, err
	}
	result.Channels = make([]AdvancedSecurityStatBucket, 0, assistantErrorReviewBucketLimit)
	result.Models = make([]AdvancedSecurityStatBucket, 0, assistantErrorReviewBucketLimit)

	var channels []struct {
		ChannelID int   `gorm:"column:channel_id"`
		Count     int64 `gorm:"column:count"`
	}
	if err := query.Session(&gorm.Session{}).Select("channel_id, COUNT(*) AS count").
		Group("channel_id").Order("count DESC, channel_id ASC").
		Limit(assistantErrorReviewBucketLimit).Scan(&channels).Error; err != nil {
		return AssistantErrorLogReview{}, err
	}
	for _, row := range channels {
		key := "unknown"
		if row.ChannelID > 0 {
			key = strconv.Itoa(row.ChannelID)
		}
		result.Channels = append(result.Channels, AdvancedSecurityStatBucket{Key: key, Count: row.Count})
	}

	var models []struct {
		ModelName string `gorm:"column:model_name"`
		Count     int64  `gorm:"column:count"`
	}
	if err := query.Session(&gorm.Session{}).Select("model_name, COUNT(*) AS count").
		Group("model_name").Order("count DESC, model_name ASC").
		Limit(assistantErrorReviewBucketLimit).Scan(&models).Error; err != nil {
		return AssistantErrorLogReview{}, err
	}
	for _, row := range models {
		key := row.ModelName
		if key == "" {
			key = "unknown"
		}
		result.Models = append(result.Models, AdvancedSecurityStatBucket{Key: key, Count: row.Count})
	}
	return result, nil
}
