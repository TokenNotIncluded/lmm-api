package model

import (
	"context"
	"testing"

	"github.com/stretchr/testify/require"
)

func TestPurgeAssistantRequestReviewsBeforeIsBounded(t *testing.T) {
	setupConsoleActivationTestDB(t)
	require.NoError(t, DB.AutoMigrate(&AssistantRequestReview{}, &AssistantReviewReset{}))
	require.NoError(t, DB.Create(&[]AssistantRequestReview{
		{UserID: 1, Status: AssistantRequestReviewStatusCompleted, CreatedAt: 1, UpdatedAt: 1},
		{UserID: 2, Status: AssistantRequestReviewStatusCompleted, CreatedAt: 2, UpdatedAt: 2},
		{UserID: 3, Status: AssistantRequestReviewStatusFailed, CreatedAt: 3, UpdatedAt: 3},
		{UserID: 4, Status: AssistantRequestReviewStatusCompleted, CreatedAt: 11, UpdatedAt: 11},
	}).Error)

	removed, err := PurgeAssistantRequestReviewsBefore(context.Background(), 10, 2)
	require.NoError(t, err)
	require.EqualValues(t, 2, removed)
	removed, err = PurgeAssistantRequestReviewsBefore(context.Background(), 10, 2)
	require.NoError(t, err)
	require.EqualValues(t, 1, removed)
	removed, err = PurgeAssistantRequestReviewsBefore(context.Background(), 10, 2)
	require.NoError(t, err)
	require.Zero(t, removed)

	var remaining int64
	require.NoError(t, DB.Model(&AssistantRequestReview{}).Count(&remaining).Error)
	require.EqualValues(t, 1, remaining)
}
