package service

import (
	"context"
	"strconv"
	"testing"
	"time"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/LIghtJUNction/api.lmm.best/model"
	"github.com/LIghtJUNction/api.lmm.best/setting"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestAssistantRetentionHandlerUsesConfiguredScheduleAndPrivacySafePayload(t *testing.T) {
	original := setting.GetAssistantSettings()
	t.Cleanup(func() {
		setting.SetAssistantRetentionEnabled(original.RetentionEnabled)
		_ = setting.UpdateAssistantActiveRetentionDays(strconv.Itoa(original.ActiveRetentionDays))
		_ = setting.UpdateAssistantArchivedRetentionDays(strconv.Itoa(original.ArchivedRetentionDays))
		_ = setting.UpdateAssistantSecurityRetentionDays(strconv.Itoa(original.SecurityRetentionDays))
		_ = setting.UpdateAssistantRetentionIntervalHours(strconv.Itoa(original.RetentionIntervalHours))
	})

	setting.SetAssistantRetentionEnabled(true)
	require.NoError(t, setting.UpdateAssistantActiveRetentionDays("120"))
	require.NoError(t, setting.UpdateAssistantArchivedRetentionDays("45"))
	require.NoError(t, setting.UpdateAssistantSecurityRetentionDays("365"))
	require.NoError(t, setting.UpdateAssistantRetentionIntervalHours("12"))

	handler := assistantRetentionHandler{}
	assert.True(t, handler.Enabled())
	assert.Equal(t, 12*time.Hour, handler.Interval())
	payload, ok := handler.NewPayload().(AssistantRetentionPayload)
	require.True(t, ok)
	assert.Equal(t, assistantRetentionBatchSize, payload.BatchSize)
	now := time.Now().Unix()
	assert.InDelta(t, now-int64(120*24*time.Hour/time.Second), payload.ActiveBefore, 2)
	assert.InDelta(t, now-int64(45*24*time.Hour/time.Second), payload.ArchivedBefore, 2)
	assert.InDelta(t, now-int64(365*24*time.Hour/time.Second), payload.RestrictedBefore, 2)
}

func TestAssistantRetentionHandlerDeletesInBatchesAndFinishesTask(t *testing.T) {
	truncate(t)
	require.NoError(t, model.DB.AutoMigrate(
		&model.AssistantConversation{},
		&model.AssistantHistoryMessage{},
		&model.AssistantSecureCard{},
		&model.AssistantSecurityIncident{},
		&model.UnifiedTodoRead{},
	))
	t.Cleanup(func() {
		model.DB.Exec("DELETE FROM unified_todo_reads")
		model.DB.Exec("DELETE FROM assistant_security_incidents")
		model.DB.Exec("DELETE FROM assistant_secure_cards")
		model.DB.Exec("DELETE FROM assistant_history_messages")
		model.DB.Exec("DELETE FROM assistant_conversations")
	})

	for index := range 3 {
		conversation := model.AssistantConversation{
			UserId:             100 + index,
			Title:              "old",
			LastMessagePreview: "old",
			CreatedAt:          1,
			UpdatedAt:          1,
		}
		require.NoError(t, model.DB.Create(&conversation).Error)
		require.NoError(t, model.DB.Create(&model.AssistantHistoryMessage{
			ConversationId: conversation.Id,
			Sequence:       1,
			Role:           model.AssistantHistoryRoleUser,
			Content:        "redacted",
			CreatedAt:      1,
		}).Error)
	}

	payload := AssistantRetentionPayload{
		AssistantRetentionCutoffs: model.AssistantRetentionCutoffs{
			ActiveBefore:     10,
			ArchivedBefore:   10,
			RestrictedBefore: 10,
		},
		BatchSize: 2,
	}
	task, err := model.CreateSystemTask(model.SystemTaskTypeAssistantRetention, payload, AssistantRetentionState{})
	require.NoError(t, err)
	claimedTask, claimed, err := model.ClaimSystemTask(task.ID, task.Type, "retention-runner", common.GetTimestamp()+60)
	require.NoError(t, err)
	require.True(t, claimed)

	assistantRetentionHandler{}.Run(context.Background(), claimedTask, "retention-runner")

	stored, err := model.GetSystemTaskByTaskID(task.TaskID)
	require.NoError(t, err)
	require.NotNil(t, stored)
	assert.Equal(t, model.SystemTaskStatusSucceeded, stored.Status)
	state := AssistantRetentionState{}
	require.NoError(t, stored.DecodeState(&state))
	assert.EqualValues(t, 3, state.Conversations)
	assert.EqualValues(t, 3, state.Messages)
	assert.Equal(t, 100, state.Progress)
	var remaining int64
	require.NoError(t, model.DB.Model(&model.AssistantConversation{}).Count(&remaining).Error)
	assert.Zero(t, remaining)
}
