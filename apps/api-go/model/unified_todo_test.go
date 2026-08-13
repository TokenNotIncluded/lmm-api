package model

import (
	"testing"

	"github.com/QuantumNous/new-api/common"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestUnifiedTodoIncludesSubmittedBountyForOwner(t *testing.T) {
	db := setupOpenSourceBountyTestDB(t)
	require.NoError(t, db.AutoMigrate(&UnifiedTodoRead{}, &DeveloperAccessRequest{}, &AccountActionRequest{}))

	owner := createOpenSourceBountyUser(t, db, "todo-owner", 10_000, common.RoleCommonUser)
	participant := createOpenSourceBountyUser(t, db, "todo-participant", 0, common.RoleCommonUser)
	now := common.GetTimestamp()
	project := OpenSourceBountyProject{
		OwnerUserId: owner.Id, RepositoryUrl: "https://github.com/example/todo-bounty",
		Title: "Review the submitted fix", Description: "A reproducible bug.", Rules: "Include tests.",
		RewardQuota: 1_000, NetRewardQuota: 1_000, RewardSlots: 1, EscrowQuota: 1_000,
		Status: OpenSourceBountyStatusPublished, CreatedAt: now, UpdatedAt: now, PublishedAt: now,
	}
	require.NoError(t, db.Create(&project).Error)
	challenge := OpenSourceBountyChallenge{
		ProjectId: project.Id, ParticipantUserId: participant.Id, GithubHandle: "todo-participant",
		Status: OpenSourceBountyChallengeSubmitted, IssueUrl: "https://github.com/example/todo-bounty/issues/1",
		PullRequestUrl: "https://github.com/example/todo-bounty/pull/2", SubmissionNote: "Tests are green.",
		RewardQuota: 1_000, AcceptedAt: now - 10, SubmittedAt: now, CreatedAt: now - 10, UpdatedAt: now,
	}
	require.NoError(t, db.Create(&challenge).Error)

	page, err := GetUnifiedTodoCenter(owner.Id, owner.Role, UnifiedTodoCategoryBountyReview, 1, 20)
	require.NoError(t, err)
	require.Len(t, page.Items, 1)
	assert.Equal(t, int64(1), page.Total)
	assert.Equal(t, int64(1), page.UnreadCount)
	assert.Equal(t, challenge.Id, page.Items[0].SourceId)
	assert.Equal(t, project.Id, page.Items[0].Details["project_id"])
	assert.Equal(t, participant.Username, page.Items[0].Details["participant_username"])

	participantPage, err := GetUnifiedTodoCenter(participant.Id, participant.Role, UnifiedTodoCategoryBountyReview, 1, 20)
	require.NoError(t, err)
	assert.Empty(t, participantPage.Items)

	marked, err := MarkUnifiedTodoReads(owner.Id, owner.Role, UnifiedTodoCategoryBountyReview, []int{challenge.Id}, false)
	require.NoError(t, err)
	assert.Equal(t, 1, marked)
	page, err = GetUnifiedTodoCenter(owner.Id, owner.Role, UnifiedTodoCategoryBountyReview, 1, 20)
	require.NoError(t, err)
	assert.Equal(t, int64(0), page.UnreadCount)
	assert.True(t, page.Items[0].Read)

	require.NoError(t, db.Model(&challenge).Updates(map[string]any{
		"status": OpenSourceBountyChallengeApproved, "reviewed_at": now + 1, "updated_at": now + 1,
	}).Error)
	page, err = GetUnifiedTodoCenter(owner.Id, owner.Role, UnifiedTodoCategoryBountyReview, 1, 20)
	require.NoError(t, err)
	assert.Empty(t, page.Items)
	assert.Zero(t, page.Total)
}
