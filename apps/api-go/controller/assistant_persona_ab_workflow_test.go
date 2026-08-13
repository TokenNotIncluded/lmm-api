package controller

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/QuantumNous/new-api/common"
	"github.com/QuantumNous/new-api/model"
	"github.com/QuantumNous/new-api/setting"
	"github.com/gin-gonic/gin"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestPersonaABPolicyChain(t *testing.T) {
	tests := []struct {
		name          string
		message       string
		wantIntent    string
		wantProfile   assistantCustomerProfile
		firstTool     string
		called        map[string]bool
		nextTool      string
		promptText    string
		forbiddenTool string
	}{
		{
			name:          "A exact facts without relay or payment sales",
			message:       "我技术很强，明确讨厌中转和法币付款，只接受免费开源自建；请查真实 Base URL、可用模型和 gpt-5.6-sol 的准确价格供我比较。",
			wantIntent:    model.AssistantIntentCost,
			wantProfile:   assistantProfileTechnical,
			firstTool:     "get_service_facts",
			called:        map[string]bool{"get_service_facts": true},
			nextTool:      "get_available_models",
			promptText:    "Treat explicit free, self-hosted, open-source, no-payment, and no-relay constraints as hard requirements",
			forbiddenTool: "get_plan_offers",
		},
		{
			name:          "B guided start without premature offer",
			message:       "我愿意为稳定体验付费，但完全不懂技术；我已经说过我是新手，请一步一步告诉我从哪里开始。",
			wantIntent:    model.AssistantIntentOnboarding,
			wantProfile:   assistantProfileGuided,
			firstTool:     "get_account_access",
			called:        map[string]bool{"get_account_access": true},
			nextTool:      "",
			promptText:    "Treat the user's stated experience level as already answered",
			forbiddenTool: "get_plan_offers",
		},
	}

	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			context := assistantUserContextForRequest(0, test.message)
			assert.Equal(t, test.wantIntent, context.Intent)
			assert.Equal(t, test.wantProfile, context.CustomerProfile)

			encoded, err := json.Marshal(context)
			require.NoError(t, err)
			assert.Contains(t, string(encoded), test.promptText)
			prompt := buildAssistantSystemPrompt(setting.GetAssistantSettings(), assistantUserContext{
				UserID:          1,
				AccessLevel:     context.AccessLevel,
				CustomerProfile: context.CustomerProfile,
			})
			assert.Contains(t, prompt, test.promptText)

			assert.Equal(t, test.firstTool, assistantNamedToolChoiceName(assistantToolChoiceForAgentStep(context, nil, nil)))
			assert.Equal(t, test.nextTool, assistantNamedToolChoiceName(assistantToolChoiceForAgentStep(context, test.called, test.called)))
			assert.False(t, assistantToolAllowedForContext(test.forbiddenTool, context))
		})
	}
}

func TestPersonaAAgentChain(t *testing.T) {
	gin.SetMode(gin.TestMode)
	db := setupTokenControllerTestDB(t)
	require.NoError(t, db.AutoMigrate(
		&model.User{}, &model.TopUp{}, &model.Ability{}, &model.UserOAuthBinding{},
		&model.AssistantUserProfile{}, &model.DeveloperAccessRequest{},
	))
	user := model.User{
		Username: "persona-a-live-facts", Password: "password", Role: common.RoleCommonUser,
		Status: common.UserStatusEnabled, Group: "default",
	}
	require.NoError(t, db.Create(&user).Error)
	require.NoError(t, db.Create(&model.Ability{Group: "default", Model: "gpt-5.6-sol", ChannelId: 1, Enabled: true}).Error)
	originalGetPricing := getPricingCache
	getPricingCache = func() []model.Pricing {
		return []model.Pricing{{
			ModelName: "gpt-5.6-sol", QuotaType: 0, ModelRatio: 1.25,
			CompletionRatio: 4, EnableGroup: []string{"default"},
		}}
	}
	t.Cleanup(func() { getPricingCache = originalGetPricing })

	message := "我技术很强，明确讨厌中转和法币付款，只接受免费开源自建；请查真实 Base URL、可用模型和 gpt-5.6-sol 的准确价格供我比较。"
	context := assistantUserContextForRequest(user.Id, message)
	context.ConversationTitleNeeded = true

	recorder := httptest.NewRecorder()
	c, _ := gin.CreateTestContext(recorder)
	c.Request = httptest.NewRequest(http.MethodPost, "/api/assistant/chat", nil)
	c.Set("id", user.Id)
	c.Set(assistantActorUserIDKey, user.Id)
	c.Set(assistantUserContextKey, context)

	turn := 0
	originalRelay := relayAssistantAgentTurn
	relayAssistantAgentTurn = func(_ *gin.Context, request assistantOpenAIRequest, _ string, _ int) (int, []byte, error) {
		turn++
		switch turn {
		case 1:
			assert.Equal(t, "set_conversation_title", assistantNamedToolChoiceName(request.ToolChoice))
			return http.StatusOK, []byte(`{"choices":[{"message":{"role":"assistant","tool_calls":[{"id":"title","type":"function","function":{"name":"set_conversation_title","arguments":"{\"title\":\"自建模型价格核对\"}"}}]}}]}`), nil
		case 2:
			assert.Equal(t, "get_service_facts", assistantNamedToolChoiceName(request.ToolChoice))
			return http.StatusOK, []byte(`{"choices":[{"message":{"role":"assistant","tool_calls":[{"id":"facts","type":"function","function":{"name":"get_service_facts","arguments":"{}"}}]}}]}`), nil
		case 3:
			assert.Equal(t, "get_available_models", assistantNamedToolChoiceName(request.ToolChoice))
			return http.StatusOK, []byte(`{"choices":[{"message":{"role":"assistant","tool_calls":[{"id":"models","type":"function","function":{"name":"get_available_models","arguments":"{}"}}]}}]}`), nil
		case 4:
			assert.Equal(t, "get_model_pricing", assistantNamedToolChoiceName(request.ToolChoice))
			return http.StatusOK, []byte(`{"choices":[{"message":{"role":"assistant","tool_calls":[{"id":"pricing","type":"function","function":{"name":"get_model_pricing","arguments":"{\"model_id\":\"gpt-5.6-sol\"}"}}]}}]}`), nil
		case 5:
			assert.Nil(t, request.ToolChoice)
			assert.Empty(t, request.Tools)
			encoded := string(mustAssistantJSON(t, request.Messages))
			assert.Contains(t, encoded, `\"model_ids\":[\"gpt-5.6-sol\"]`)
			assert.Contains(t, encoded, `\"pricing_scope\":\"public_preview_reference\"`)
			return http.StatusOK, []byte(`{"choices":[{"message":{"role":"assistant","content":"已核对实时连接、模型目录和参考价格；不会推荐中转、付费或法币路径。"}}]}`), nil
		default:
			return http.StatusInternalServerError, nil, nil
		}
	}
	t.Cleanup(func() { relayAssistantAgentTurn = originalRelay })

	runAssistantAgent(c, setting.AssistantSettings{
		Model: "persona-a-workflow-model", AgentLoopEnabled: true, MaxSteps: 5, TimeoutSeconds: 45,
	}, []assistantOpenAIMessage{{Role: "user", Content: message}})

	assert.Equal(t, 5, turn)
	assert.Equal(t, http.StatusOK, recorder.Code)
	assert.Contains(t, recorder.Body.String(), "不会推荐中转")
}

func TestPersonaBAgentChainKeepsGuidedStrategy(t *testing.T) {
	gin.SetMode(gin.TestMode)
	db := setupTokenControllerTestDB(t)
	require.NoError(t, db.AutoMigrate(
		&model.User{}, &model.TopUp{}, &model.UserOAuthBinding{},
		&model.AssistantUserProfile{}, &model.DeveloperAccessRequest{},
	))
	user := model.User{
		Username: "persona-b-guided", Password: "password", Role: common.RoleCommonUser,
		Status: common.UserStatusEnabled, Group: "default",
	}
	require.NoError(t, db.Create(&user).Error)

	message := "我愿意为稳定体验付费，但完全不懂技术；我已经说过我是新手，请一步一步告诉我从哪里开始。"
	context := assistantUserContextForRequest(user.Id, message)
	require.Equal(t, assistantProfileGuided, context.CustomerProfile)
	require.Equal(t, assistantPaymentOfferNeedsDetails, context.PaymentOfferState)

	recorder := httptest.NewRecorder()
	c, _ := gin.CreateTestContext(recorder)
	c.Request = httptest.NewRequest(http.MethodPost, "/api/assistant/chat", nil)
	c.Set("id", user.Id)
	c.Set(assistantActorUserIDKey, user.Id)
	c.Set(assistantUserContextKey, context)

	turn := 0
	originalRelay := relayAssistantAgentTurn
	relayAssistantAgentTurn = func(_ *gin.Context, request assistantOpenAIRequest, _ string, _ int) (int, []byte, error) {
		turn++
		switch turn {
		case 1:
			assert.Equal(t, "get_account_access", assistantNamedToolChoiceName(request.ToolChoice))
			encoded := string(mustAssistantJSON(t, request))
			assert.Contains(t, encoded, "Treat the user's stated experience level as already answered")
			assert.NotContains(t, encoded, `"name":"get_plan_offers"`)
			return http.StatusOK, []byte(`{"choices":[{"message":{"role":"assistant","tool_calls":[{"id":"access","type":"function","function":{"name":"get_account_access","arguments":"{}"}}]}}]}`), nil
		case 2:
			assert.Nil(t, request.ToolChoice)
			assert.Empty(t, request.Tools)
			return http.StatusOK, []byte(`{"choices":[{"message":{"role":"assistant","content":"1. 先确认你要使用的客户端。你已经说明自己是新手，我不会再重复询问；在用途和金额明确前也不会展示付费方案。"}}]}`), nil
		default:
			return http.StatusInternalServerError, nil, nil
		}
	}
	t.Cleanup(func() { relayAssistantAgentTurn = originalRelay })

	runAssistantAgent(c, setting.AssistantSettings{
		Model: "persona-b-workflow-model", AgentLoopEnabled: true, MaxSteps: 2, TimeoutSeconds: 45,
	}, []assistantOpenAIMessage{{Role: "user", Content: message}})

	assert.Equal(t, 2, turn)
	assert.Equal(t, http.StatusOK, recorder.Code)
	assert.Contains(t, recorder.Body.String(), "不会再重复询问")
}
