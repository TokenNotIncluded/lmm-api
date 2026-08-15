package model

import (
	"context"
	"testing"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/stretchr/testify/require"
)

func TestCanViewAssistantReviewViolationsUsesStrictRoleBoundary(t *testing.T) {
	owner := &User{Id: 10, Role: common.RoleAdminUser}
	tests := []struct {
		name   string
		viewer int
		role   int
		want   bool
	}{
		{name: "owner", viewer: 10, role: common.RoleCommonUser, want: true},
		{name: "ordinary other user", viewer: 11, role: common.RoleCommonUser, want: false},
		{name: "lower administrator", viewer: 11, role: common.RoleAdminUser, want: false},
		{name: "root can inspect lower administrator", viewer: 11, role: common.RoleRootUser, want: true},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			require.Equal(t, test.want, canViewAssistantReviewViolations(test.viewer, test.role, owner))
		})
	}
}

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
