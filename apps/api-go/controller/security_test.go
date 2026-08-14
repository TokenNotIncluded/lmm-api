package controller

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/QuantumNous/new-api/setting"
	"github.com/gin-gonic/gin"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestSecurityPolicySeparatesPublicAndAdminRuleDetails(t *testing.T) {
	original := setting.GetAdvancedSecuritySettings()
	originalRules := setting.AdvancedSecurityRulesToJSONString()
	t.Cleanup(func() {
		setting.SetAdvancedSecurityEnabled(original.Enabled)
		setting.SetAdvancedSecurityOnPrompt(original.OnPrompt)
		_ = setting.UpdateAdvancedSecurityAction(original.Action)
		_ = setting.UpdateAdvancedSecurityRules(originalRules)
	})

	setting.SetAdvancedSecurityEnabled(true)
	setting.SetAdvancedSecurityOnPrompt(true)
	require.NoError(t, setting.UpdateAdvancedSecurityAction(setting.AdvancedSecurityActionBlock))
	require.NoError(t, setting.UpdateAdvancedSecurityRules(`[{"id":"prompt-injection","name":"Prompt injection","category":"prompt_injection","enabled":true,"patterns":["do not publish this matcher"]}]`))

	publicRecorder := httptest.NewRecorder()
	publicContext, _ := gin.CreateTestContext(publicRecorder)
	publicContext.Request = httptest.NewRequest(http.MethodGet, "/api/security/policy", nil)
	GetPublicSecurityPolicy(publicContext)
	require.Equal(t, http.StatusOK, publicRecorder.Code)
	assert.NotContains(t, publicRecorder.Body.String(), "do not publish this matcher")
	assert.Contains(t, publicRecorder.Body.String(), "prompt_injection")
	assert.Contains(t, publicRecorder.Body.String(), "violation_fee.usage_policy")
	assert.NotContains(t, publicRecorder.Body.String(), "Grok / xAI upstream")
	assert.Contains(t, publicRecorder.Body.String(), setting.AdvancedSecurityPolicyReferenceDate)

	adminRecorder := httptest.NewRecorder()
	adminContext, _ := gin.CreateTestContext(adminRecorder)
	adminContext.Request = httptest.NewRequest(http.MethodGet, "/api/security/admin/policy", nil)
	GetAdminSecurityPolicy(adminContext)
	require.Equal(t, http.StatusOK, adminRecorder.Code)
	assert.Contains(t, adminRecorder.Body.String(), "do not publish this matcher")

	var payload struct {
		Data struct {
			Public struct {
				Enforcement struct {
					Enabled  bool   `json:"enabled"`
					OnPrompt bool   `json:"on_prompt"`
					Action   string `json:"action"`
				} `json:"enforcement"`
				Rules []struct {
					Patterns []string `json:"patterns"`
				} `json:"rules"`
			} `json:"public"`
			Rules []struct {
				Patterns []string `json:"patterns"`
			} `json:"rules"`
		} `json:"data"`
	}
	require.NoError(t, json.Unmarshal(adminRecorder.Body.Bytes(), &payload))
	assert.True(t, payload.Data.Public.Enforcement.Enabled)
	assert.True(t, payload.Data.Public.Enforcement.OnPrompt)
	assert.Equal(t, setting.AdvancedSecurityActionBlock, payload.Data.Public.Enforcement.Action)
	require.Len(t, payload.Data.Public.Rules, 1)
	assert.Empty(t, payload.Data.Public.Rules[0].Patterns)
	require.Len(t, payload.Data.Rules, 1)
	assert.Equal(t, []string{"do not publish this matcher"}, payload.Data.Rules[0].Patterns)
}
