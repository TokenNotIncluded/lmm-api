package relay

import (
	"encoding/json"
	"net/http/httptest"
	"testing"

	"github.com/LIghtJUNction/api.lmm.best/model"
	relaycommon "github.com/LIghtJUNction/api.lmm.best/relay/common"
	"github.com/LIghtJUNction/api.lmm.best/setting"
	"github.com/gin-gonic/gin"
	"github.com/glebarez/sqlite"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
	"gorm.io/gorm"
)

func TestCheckAdvancedSecurityTaskPromptBlocksBeforeUpstream(t *testing.T) {
	originalSettings := setting.GetAdvancedSecuritySettings()
	originalDB := model.DB
	t.Cleanup(func() {
		model.DB = originalDB
		setting.SetAdvancedSecurityEnabled(originalSettings.Enabled)
		setting.SetAdvancedSecurityOnPrompt(originalSettings.OnPrompt)
		require.NoError(t, setting.UpdateAdvancedSecurityAction(originalSettings.Action))
		require.NoError(t, setting.UpdateAdvancedSecurityRules(advancedSecurityRulesJSONForTest(originalSettings)))
	})

	db, err := gorm.Open(sqlite.Open("file:relay-task-security?mode=memory&cache=shared"), &gorm.Config{})
	require.NoError(t, err)
	require.NoError(t, db.AutoMigrate(&model.AdvancedSecurityEvent{}))
	model.DB = db

	setting.SetAdvancedSecurityEnabled(true)
	setting.SetAdvancedSecurityOnPrompt(true)
	require.NoError(t, setting.UpdateAdvancedSecurityAction(setting.AdvancedSecurityActionBlock))
	require.NoError(t, setting.UpdateAdvancedSecurityRules(`[{"id":"task-rule","category":"computer_network_compromise","enabled":true,"groups":["default"],"patterns":["blocked task prompt"]}]`))

	c, _ := gin.CreateTestContext(httptest.NewRecorder())
	c.Request = httptest.NewRequest("POST", "/v1/videos", nil)
	c.Set("id", 42)
	c.Set("task_request", relaycommon.TaskSubmitReq{Prompt: "please use a blocked task prompt now"})

	taskErr := checkAdvancedSecurityTaskPrompt(c, &relaycommon.RelayInfo{UserId: 42, UserGroup: "default", UsingGroup: "default"})
	require.NotNil(t, taskErr)
	assert.Equal(t, "advanced_security_guardrail", taskErr.Code)
	assert.Equal(t, 400, taskErr.StatusCode)

	var events []model.AdvancedSecurityEvent
	require.NoError(t, db.Find(&events).Error)
	require.Len(t, events, 1)
	assert.Equal(t, model.AdvancedSecurityDecisionBlocked, events[0].Decision)
	assert.NotEmpty(t, events[0].InputDigest)
	assert.NotContains(t, events[0].InputDigest, "blocked task prompt")
}

func TestTaskPromptFromContextSupportsSunoAndStandardRequests(t *testing.T) {
	c, _ := gin.CreateTestContext(httptest.NewRecorder())
	c.Set("task_request", relaycommon.TaskSubmitReq{Prompt: "standard prompt"})
	prompt, ok := taskPromptFromContext(c)
	assert.True(t, ok)
	assert.Equal(t, "standard prompt", prompt)

	c.Set("task_request", &struct{ Prompt string }{Prompt: "unsupported request"})
	_, ok = taskPromptFromContext(c)
	assert.False(t, ok)
}

func advancedSecurityRulesJSONForTest(settings setting.AdvancedSecuritySettings) string {
	encoded, err := json.Marshal(settings.RuleSet)
	if err != nil {
		panic(err)
	}
	return string(encoded)
}
