package controller

import (
	"testing"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/LIghtJUNction/api.lmm.best/model"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestAssistantBountyDataToolSharesMCPReadBoundary(t *testing.T) {
	db, owner, _ := setupOpenSourceBountyMCPControllerTest(t)
	project, err := model.CreateOpenSourceBountyDraft(owner.Id, model.OpenSourceBountyDraftInput{
		RepositoryUrl: "https://github.com/example/assistant-bounty",
		Title:         "Assistant read boundary",
		Description:   "A public bounty used to verify assistant reads.",
		Rules:         "Provide reproducible evidence and a focused fix.",
		RewardQuota:   100,
		RewardSlots:   1,
	})
	require.NoError(t, err)
	_, _, err = model.PublishOpenSourceBounty(owner.Id, project.Id)
	require.NoError(t, err)
	participantLevel := model.TrustLevelMinUser + 1
	participant := model.User{
		Username: "assistant-bounty-participant", Password: "password", Role: common.RoleCommonUser,
		Status: common.UserStatusEnabled, TrustLevelOverride: &participantLevel,
	}
	require.NoError(t, db.Create(&participant).Error)
	_, err = model.AcceptOpenSourceBounty(participant.Id, project.Id, "assistant-participant")
	require.NoError(t, err)

	board := executeAssistantBountyDataTool(owner.Id, map[string]any{"view": "board", "page_size": float64(1)})
	assert.Equal(t, true, board["ok"])
	boardData, ok := board["data"].(map[string]any)
	require.True(t, ok)
	boardItems, ok := boardData["items"].([]model.OpenSourceBountyProjectView)
	require.True(t, ok)
	require.Len(t, boardItems, 1)
	assert.Equal(t, project.Id, boardItems[0].Id)

	detail := executeAssistantBountyDataTool(owner.Id, map[string]any{"view": "detail", "project_id": float64(project.Id)})
	assert.Equal(t, true, detail["ok"])
	detailValue, ok := detail["data"].(*model.OpenSourceBountyProjectDetail)
	require.True(t, ok)
	assert.Equal(t, project.Id, detailValue.Project.Id)

	owned := executeAssistantBountyDataTool(owner.Id, map[string]any{"view": "owned"})
	assert.Equal(t, true, owned["ok"])
	ownedItems, ok := owned["data"].([]model.OpenSourceBountyProjectView)
	require.True(t, ok)
	require.Len(t, ownedItems, 1)
	assert.Equal(t, project.Id, ownedItems[0].Id)

	accepted := executeAssistantBountyDataTool(participant.Id, map[string]any{"view": "accepted"})
	assert.Equal(t, true, accepted["ok"])
	acceptedItems, ok := accepted["data"].([]model.OpenSourceBountyChallengeView)
	require.True(t, ok)
	require.Len(t, acceptedItems, 1)
	assert.Equal(t, project.Id, acceptedItems[0].ProjectId)

	disputes := executeAssistantBountyDataTool(owner.Id, map[string]any{"view": "disputes", "limit": float64(1)})
	assert.Equal(t, true, disputes["ok"])
	disputeItems, ok := disputes["data"].([]model.OpenSourceBountyDisputeView)
	require.True(t, ok)
	assert.Empty(t, disputeItems)

	privateL0 := model.TrustLevelMinUser
	l0 := model.User{
		Username: "assistant-bounty-l0", Password: "password", Role: common.RoleCommonUser,
		Status: common.UserStatusEnabled, AffCode: "assistant-bounty-l0", TrustLevelOverride: &privateL0,
	}
	require.NoError(t, db.Create(&l0).Error)
	redacted := executeAssistantBountyDataTool(l0.Id, map[string]any{"view": "owned"})
	assert.Equal(t, false, redacted["ok"])
	assert.Equal(t, "l1_required", redacted["status"])
	assert.NotContains(t, redacted, "data")

	assert.True(t, assistantToolAllowedForContext("get_bounty_data", assistantUserContext{
		Intent: model.AssistantIntentBounty, AccessLevel: "L0",
	}))
	assert.False(t, assistantToolAllowedForContext("get_bounty_data", assistantUserContext{AccessLevel: "L0"}))
	assert.Equal(t, "get_bounty_data", assistantNamedToolChoiceName(assistantToolChoiceForContext(assistantUserContext{
		Intent: model.AssistantIntentBounty, LatestUserRequest: "查看悬赏列表", AccessLevel: "L0",
	})))
	assert.Equal(t, "get_bounty_guide", assistantNamedToolChoiceName(assistantToolChoiceForContext(assistantUserContext{
		Intent: model.AssistantIntentBounty, LatestUserRequest: "如何发布悬赏", AccessLevel: "L0",
	})))
}

func TestAssistantBountyDataToolRejectsMutationAndBoundsInput(t *testing.T) {
	result := executeAssistantBountyDataTool(1, map[string]any{"view": "publish"})
	assert.Equal(t, false, result["ok"])
	assert.Equal(t, "invalid_input", result["status"])

	result = executeAssistantBountyDataTool(1, map[string]any{"view": "board", "page_size": float64(51)})
	assert.Equal(t, false, result["ok"])
	assert.Equal(t, "invalid_input", result["status"])
}
