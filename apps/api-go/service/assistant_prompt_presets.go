package service

import (
	"context"
	"time"

	"github.com/LIghtJUNction/api.lmm.best/model"
	"github.com/LIghtJUNction/api.lmm.best/setting"
)

const promptPresetRefreshInterval = 6 * time.Hour

type promptPresetTask struct{}

func (promptPresetTask) Type() string {
	return model.SystemTaskTypeAssistantPresets
}

func (promptPresetTask) Enabled() bool {
	return setting.GetAssistantSettings().Enabled
}

func (promptPresetTask) Interval() time.Duration {
	return promptPresetRefreshInterval
}

func (promptPresetTask) NewPayload() any {
	return nil
}

func (promptPresetTask) Run(ctx context.Context, task *model.SystemTask, runnerID string) {
	if err := ctx.Err(); err != nil {
		failSystemTask(task, runnerID, err)
		return
	}
	result, err := model.RefreshPromptPresets()
	if err != nil {
		failSystemTask(task, runnerID, err)
		return
	}
	if err := model.FinishSystemTask(task.TaskID, runnerID, model.SystemTaskStatusSucceeded, result, ""); err != nil {
		logSystemTaskLockError(ctx, task, err)
	}
}

func init() {
	RegisterSystemTaskHandler(promptPresetTask{})
}
