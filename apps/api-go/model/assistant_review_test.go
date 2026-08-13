package model

import (
	"context"
	"encoding/json"
	"strings"
	"testing"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestReviewAggregates(t *testing.T) {
	user := setupAssistantLeadTestDB(t)
	require.NoError(t, DB.AutoMigrate(&PromptPresetStat{}, &AssistantSecurityIncident{}))

	require.NoError(t, DB.Create(&AssistantProfileBucket{Profile: AssistantProfileUnknown, BucketStart: 100, Count: 5}).Error)
	require.NoError(t, DB.Create(&AssistantProfileBucket{Profile: AssistantProfileTechnical, BucketStart: 100, Count: 2}).Error)
	require.NoError(t, DB.Create(&AssistantProfileBucket{Profile: AssistantProfileUnknown, BucketStart: 300, Count: 99}).Error)
	require.NoError(t, DB.Create(&AssistantLead{
		UserId: user.Id, Source: AssistantLeadSourceHandoff, Intent: AssistantIntentHumanSupport,
		Message: "private support text", Status: AssistantLeadStatusPending, CreatedAt: 100,
	}).Error)
	require.NoError(t, DB.Create(&PromptPresetStat{
		PresetId: "pricing_cost", BucketStart: 100, Generation: 1, Version: PromptPresetVersion,
		ClickCount: 10, ConversationCount: 2, RecommendationCount: 4, ApprovalCount: 0, UpdatedAt: 100,
	}).Error)
	require.NoError(t, DB.Create(&PromptPresetStat{
		PresetId: "pricing_cost", BucketStart: 300, Generation: 2, Version: PromptPresetVersion,
		ClickCount: 99, ConversationCount: 99, UpdatedAt: 300,
	}).Error)
	require.NoError(t, DB.Create(&AssistantSecurityIncident{
		UserId: user.Id, ConversationId: 99, Category: AssistantSecurityIncidentCategory,
		Status: AssistantSecurityIncidentStatusOpen, InputDigest: strings.Repeat("a", 64), CreatedAt: 100, UpdatedAt: 100,
	}).Error)

	review, err := BuildAssistantReview(context.Background(), 1, 200)
	require.NoError(t, err)
	assert.EqualValues(t, 1, review.CurrentSupport)
	assert.EqualValues(t, 1, review.CurrentIncidents)
	require.Len(t, review.Presets, 1)
	assert.EqualValues(t, 10, review.Presets[0].Clicks)
	assert.EqualValues(t, 5, review.Profiles[0].Count)

	codes := make([]string, 0, len(review.Actions))
	for _, action := range review.Actions {
		codes = append(codes, action.Code)
	}
	assert.ElementsMatch(t, []string{
		"review_support_queue",
		"review_security_incidents",
		"improve_profile_classification",
		"improve_pre_conversation_prompts",
		"review_recommendation_quality",
	}, codes)

	encoded, err := json.Marshal(review)
	require.NoError(t, err)
	assert.NotContains(t, string(encoded), "private support text")
	assert.NotContains(t, string(encoded), user.Email)
	assert.Less(t, len(encoded), 16*1024)
}

func TestReviewInvalidWindow(t *testing.T) {
	_, err := BuildAssistantReview(context.Background(), 0, 1)
	require.Error(t, err)
}

func TestReviewCancellation(t *testing.T) {
	setupAssistantLeadTestDB(t)
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	_, err := BuildAssistantReview(ctx, 1, 2)
	require.ErrorIs(t, err, context.Canceled)
}
