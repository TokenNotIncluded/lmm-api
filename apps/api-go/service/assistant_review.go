package service

import (
	"context"
	"errors"
	"fmt"
	"time"

	"github.com/LIghtJUNction/api.lmm.best/logger"
	"github.com/LIghtJUNction/api.lmm.best/model"
	"github.com/LIghtJUNction/api.lmm.best/setting"
)

type AssistantReviewPayload struct {
	WindowStart int64 `json:"window_start"`
	WindowEnd   int64 `json:"window_end"`
}

const (
	reviewBucket       = time.Hour
	reviewHistoryLimit = 30
)

type assistantReviewHandler struct{}

func StartAssistantReview() (*model.SystemTask, error) {
	handler := assistantReviewHandler{}
	task, _, err := EnqueueSystemTask(handler.Type(), handler.NewPayload())
	return task, err
}

func (assistantReviewHandler) Type() string {
	return model.SystemTaskTypeAssistantReview
}

func (assistantReviewHandler) Enabled() bool {
	return setting.GetAssistantSettings().ReviewEnabled
}

func (assistantReviewHandler) Interval() time.Duration {
	hours := setting.GetAssistantSettings().ReviewIntervalHours
	if hours < 1 {
		hours = 24
	}
	return time.Duration(hours) * time.Hour
}

func (assistantReviewHandler) NewPayload() any {
	settings := setting.GetAssistantSettings()
	end := time.Now()
	start := end.Add(-time.Duration(settings.ReviewWindowDays) * 24 * time.Hour).Truncate(reviewBucket)
	return AssistantReviewPayload{
		WindowStart: start.Unix(),
		WindowEnd:   end.Unix(),
	}
}

func (assistantReviewHandler) Run(ctx context.Context, task *model.SystemTask, runnerID string) {
	if err := ctx.Err(); err != nil {
		failSystemTask(task, runnerID, err)
		return
	}
	payload := AssistantReviewPayload{}
	if err := task.DecodePayload(&payload); err != nil {
		failSystemTask(task, runnerID, err)
		return
	}
	if payload.WindowStart <= 0 || payload.WindowEnd < payload.WindowStart {
		failSystemTask(task, runnerID, errors.New("assistant review window is invalid"))
		return
	}
	review, err := model.BuildAssistantReview(ctx, payload.WindowStart, payload.WindowEnd)
	if err != nil {
		failSystemTask(task, runnerID, err)
		return
	}
	if err := model.FinishSystemTask(task.TaskID, runnerID, model.SystemTaskStatusSucceeded, review, ""); err != nil {
		logSystemTaskLockError(ctx, task, err)
		return
	}
	if err := model.SaveAssistantSecurityReviewNotice(
		task.TaskID, payload.WindowStart, payload.WindowEnd, review.Security, review.ObservedAt,
	); err != nil {
		logger.LogWarn(ctx, fmt.Sprintf("assistant security review notification failed: %v", err))
		return
	}
	if err := model.PruneTaskHistory(task.Type, reviewHistoryLimit); err != nil {
		logSystemTaskLockError(ctx, task, err)
	}
	if err := model.PruneAssistantSecurityReviewNotices(reviewHistoryLimit); err != nil {
		logSystemTaskLockError(ctx, task, err)
	}
}

func init() {
	RegisterSystemTaskHandler(assistantReviewHandler{})
}
