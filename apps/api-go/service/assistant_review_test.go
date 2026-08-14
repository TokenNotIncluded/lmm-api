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

func TestReviewSchedule(t *testing.T) {
	original := setting.GetAssistantSettings()
	t.Cleanup(func() {
		setting.SetAssistantReviewEnabled(original.ReviewEnabled)
		_ = setting.UpdateAssistantReviewWindowDays(strconv.Itoa(original.ReviewWindowDays))
		_ = setting.UpdateAssistantReviewIntervalHours(strconv.Itoa(original.ReviewIntervalHours))
	})

	setting.SetAssistantReviewEnabled(true)
	require.NoError(t, setting.UpdateAssistantReviewWindowDays("14"))
	require.NoError(t, setting.UpdateAssistantReviewIntervalHours("6"))

	handler := assistantReviewHandler{}
	assert.True(t, handler.Enabled())
	assert.Equal(t, 6*time.Hour, handler.Interval())
	payload := handler.NewPayload().(AssistantReviewPayload)
	assert.EqualValues(t, 0, payload.WindowStart%int64(time.Hour/time.Second))
	assert.InDelta(t, time.Now().Add(-14*24*time.Hour).Unix(), payload.WindowStart, float64(time.Hour/time.Second))
	assert.InDelta(t, time.Now().Unix(), payload.WindowEnd, 2)
}

func TestReviewTask(t *testing.T) {
	truncate(t)
	require.NoError(t, model.DB.AutoMigrate(
		&model.AssistantLead{}, &model.AssistantProfileBucket{}, &model.PromptPresetStat{},
		&model.AssistantSecurityIncident{}, &model.AdvancedSecurityEvent{}, &model.AssistantSecurityReviewNotice{},
		&model.TopUp{}, &model.SubscriptionOrder{}, &model.FinanceLedgerEntry{},
	))
	t.Cleanup(func() {
		model.DB.Exec("DELETE FROM assistant_security_incidents")
		model.DB.Exec("DELETE FROM assistant_pre_conversation_preset_stats")
		model.DB.Exec("DELETE FROM assistant_profile_buckets")
		model.DB.Exec("DELETE FROM assistant_leads")
		model.DB.Exec("DELETE FROM advanced_security_events")
		model.DB.Exec("DELETE FROM assistant_security_review_notices")
	})
	require.NoError(t, model.DB.Create(&model.AdvancedSecurityEvent{
		CreatedAt: 50, RequestID: "review-request", UserID: 42,
		Decision: model.AdvancedSecurityDecisionBlocked, RuleID: "review-rule", Category: "review-category",
	}).Error)

	payload := AssistantReviewPayload{WindowStart: 1, WindowEnd: 100}
	task, err := model.CreateSystemTask(model.SystemTaskTypeAssistantReview, payload, nil)
	require.NoError(t, err)
	claimed, ok, err := model.ClaimSystemTask(task.ID, task.Type, "review-runner", common.GetTimestamp()+60)
	require.NoError(t, err)
	require.True(t, ok)

	assistantReviewHandler{}.Run(context.Background(), claimed, "review-runner")
	stored, err := model.GetSystemTaskByTaskID(task.TaskID)
	require.NoError(t, err)
	assert.Equal(t, model.SystemTaskStatusSucceeded, stored.Status)
	assert.Contains(t, stored.Result, `"window_start":1`)
	assert.Contains(t, stored.Result, `"review_security_events"`)
	assert.Less(t, len(stored.Result), 16*1024)
	var notices []model.AssistantSecurityReviewNotice
	require.NoError(t, model.DB.Find(&notices).Error)
	assert.Len(t, notices, 1)
}
