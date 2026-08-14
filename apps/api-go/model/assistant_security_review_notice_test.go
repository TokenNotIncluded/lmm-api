package model

import (
	"encoding/json"
	"strconv"
	"strings"
	"testing"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestAssistantSecurityReviewNoticeSaveIsIdempotentAndAggregateOnly(t *testing.T) {
	db := setupConsoleActivationTestDB(t)
	require.NoError(t, db.AutoMigrate(&AssistantSecurityReviewNotice{}))
	review := AssistantSecurityReview{
		TotalMatches:     3,
		BlockedMatches:   2,
		AuditedMatches:   1,
		AffectedRequests: 2,
		AffectedUsers:    2,
		ByCategory:       []AdvancedSecurityStatBucket{{Key: strings.Repeat("category-", 40), Count: 3}},
		ByRule:           []AdvancedSecurityStatBucket{{Key: "prompt-injection", Count: 3}},
		ErrorLogCount:    4,
		ErrorChannels:    []AdvancedSecurityStatBucket{{Key: "7", Count: 4}},
		ErrorModels:      []AdvancedSecurityStatBucket{{Key: "gpt-review", Count: 4}},
	}
	require.NoError(t, SaveAssistantSecurityReviewNotice("review-task-1", 100, 200, review, 300))
	review.TotalMatches = 99
	review.ErrorLogCount = 99
	require.NoError(t, SaveAssistantSecurityReviewNotice("review-task-1", 100, 200, review, 301))

	var notice AssistantSecurityReviewNotice
	require.NoError(t, db.First(&notice).Error)
	assert.EqualValues(t, 3, notice.TotalMatches)
	assert.EqualValues(t, 4, notice.ErrorLogCount)
	assert.NotContains(t, notice.ByCategoryJSON, "request_id")
	assert.NotContains(t, notice.ByCategoryJSON, "user_id")
	assert.NotContains(t, notice.ByCategoryJSON, "username")
	assert.LessOrEqual(t, len([]rune(notice.ByCategoryJSON)), 20*assistantSecurityReviewKeyMax)

	aggregate, err := notice.Aggregate()
	require.NoError(t, err)
	assert.EqualValues(t, 3, aggregate.TotalMatches)
	assert.Len(t, aggregate.ByCategory, 1)
	assert.EqualValues(t, 4, aggregate.ErrorLogCount)
	assert.Equal(t, []AdvancedSecurityStatBucket{{Key: "7", Count: 4}}, aggregate.ErrorChannels)
	assert.Equal(t, []AdvancedSecurityStatBucket{{Key: "gpt-review", Count: 4}}, aggregate.ErrorModels)
	assert.LessOrEqual(t, len([]rune(aggregate.ByCategory[0].Key)), assistantSecurityReviewKeyMax)
	encoded, err := json.Marshal(notice)
	require.NoError(t, err)
	assert.NotContains(t, string(encoded), "review-task-1")
	assert.NotContains(t, string(encoded), "request_id")
}

func TestAssistantSecurityReviewNoticePruneIsBounded(t *testing.T) {
	db := setupConsoleActivationTestDB(t)
	require.NoError(t, db.AutoMigrate(&AssistantSecurityReviewNotice{}))
	for index := 0; index < assistantSecurityReviewNoticeMax+5; index++ {
		require.NoError(t, SaveAssistantSecurityReviewNotice(
			"review-task-"+strconv.Itoa(index),
			int64(index+1), int64(index+2), AssistantSecurityReview{TotalMatches: 1}, int64(index+1),
		))
	}
	require.NoError(t, PruneAssistantSecurityReviewNotices(assistantSecurityReviewNoticeKeep))
	var count int64
	require.NoError(t, db.Model(&AssistantSecurityReviewNotice{}).Count(&count).Error)
	assert.EqualValues(t, assistantSecurityReviewNoticeKeep, count)
}
