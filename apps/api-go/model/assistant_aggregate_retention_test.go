package model

import (
	"context"
	"testing"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestPurgeAssistantAggregateBucketsIsBounded(t *testing.T) {
	setupConsoleActivationTestDB(t)
	require.NoError(t, DB.AutoMigrate(&AssistantProfileBucket{}, &AssistantFirstQuestionStat{}))
	require.NoError(t, DB.Create(&[]AssistantProfileBucket{
		{Profile: "unknown", BucketStart: 1, Count: 1},
		{Profile: "technical_cost_sensitive", BucketStart: 2, Count: 2},
		{Profile: "guided_buyer", BucketStart: 3, Count: 3},
		{Profile: "normal_user", BucketStart: 11, Count: 4},
	}).Error)
	require.NoError(t, DB.Create(&[]AssistantFirstQuestionStat{
		{QuestionHash: "old-question-1", Question: "old 1", BucketStart: 1, Count: 1, LastAskedAt: 1},
		{QuestionHash: "old-question-2", Question: "old 2", BucketStart: 2, Count: 2, LastAskedAt: 2},
		{QuestionHash: "old-question-3", Question: "old 3", BucketStart: 3, Count: 3, LastAskedAt: 3},
		{QuestionHash: "new-question", Question: "new", BucketStart: 11, Count: 4, LastAskedAt: 11},
	}).Error)

	deleted, err := PurgeAssistantProfileBucketsBefore(context.Background(), 10, 2)
	require.NoError(t, err)
	assert.EqualValues(t, 2, deleted)
	deleted, err = PurgeAssistantProfileBucketsBefore(context.Background(), 10, 2)
	require.NoError(t, err)
	assert.EqualValues(t, 1, deleted)
	deleted, err = PurgeAssistantProfileBucketsBefore(context.Background(), 10, 2)
	require.NoError(t, err)
	assert.Zero(t, deleted)

	deleted, err = PurgeAssistantFirstQuestionsBefore(context.Background(), 10, 2)
	require.NoError(t, err)
	assert.EqualValues(t, 2, deleted)
	deleted, err = PurgeAssistantFirstQuestionsBefore(context.Background(), 10, 2)
	require.NoError(t, err)
	assert.EqualValues(t, 1, deleted)
	deleted, err = PurgeAssistantFirstQuestionsBefore(context.Background(), 10, 2)
	require.NoError(t, err)
	assert.Zero(t, deleted)

	var profileCount, questionCount int64
	require.NoError(t, DB.Model(&AssistantProfileBucket{}).Count(&profileCount).Error)
	require.NoError(t, DB.Model(&AssistantFirstQuestionStat{}).Count(&questionCount).Error)
	assert.EqualValues(t, 1, profileCount)
	assert.EqualValues(t, 1, questionCount)
}
