package controller

import (
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/QuantumNous/new-api/model"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestSubmitDeveloperAccessRequestWithoutAIRecommendationStillQueuesUser(t *testing.T) {
	user, engine := setupDeveloperAccessRequestControllerTest(t)

	request := httptest.NewRequest(http.MethodPost, "/request", strings.NewReader(`{
		"reason":"I need L1 access for a real integration.",
		"confirmed":true
	}`))
	request.Header.Set("Content-Type", "application/json")
	response := httptest.NewRecorder()
	engine.ServeHTTP(response, request)

	assert.Equal(t, http.StatusOK, response.Code)
	assert.Contains(t, response.Body.String(), model.DeveloperAccessRequestSourceAssistant)
	assert.NotContains(t, response.Body.String(), "AI recommendation confirmation is invalid")
	stored, err := model.GetDeveloperAccessRequest(user.Id)
	require.NoError(t, err)
	require.NotNil(t, stored)
	assert.Equal(t, model.DeveloperAccessRequestPending, stored.Status)
	assert.Equal(t, model.DeveloperAccessRequestSourceAssistant, stored.Source)
	assert.Empty(t, stored.AIRecommendation)
}

func TestSubmitDeveloperAccessRequestQueueFailureReturnsRetryableError(t *testing.T) {
	_, engine := setupDeveloperAccessRequestControllerTest(t)
	require.NoError(t, model.DB.Migrator().DropTable(&model.DeveloperAccessRequest{}))

	request := httptest.NewRequest(http.MethodPost, "/request", strings.NewReader(`{
		"reason":"I need L1 access for a real integration.",
		"confirmed":true
	}`))
	request.Header.Set("Content-Type", "application/json")
	response := httptest.NewRecorder()
	engine.ServeHTTP(response, request)

	assert.Equal(t, http.StatusServiceUnavailable, response.Code)
	assert.Equal(t, "2", response.Header().Get("Retry-After"))
	assert.Contains(t, response.Body.String(), "DEVELOPER_ACCESS_QUEUE_UNAVAILABLE")
}

func TestSubmitDeveloperAccessAIQueueFailureKeepsConfirmationReusable(t *testing.T) {
	user, engine := setupDeveloperAccessRequestControllerTest(t)
	payload := `{"user_statement":"I need L1 access for a real integration.","recommendation":"The user described a concrete integration workflow and can be reviewed for L1 access."}`
	token, _, err := model.CreateAuthFlow(model.AuthFlowCreate{
		Purpose:   model.AuthFlowPurposeAssistantL1,
		UserId:    user.Id,
		SessionId: "developer-access-test-session",
		Payload:   payload,
		ExpiresAt: time.Now().Add(time.Minute),
	})
	require.NoError(t, err)
	require.NoError(t, model.DB.Migrator().DropTable(&model.DeveloperAccessRequest{}))

	request := httptest.NewRequest(http.MethodPost, "/request", strings.NewReader(`{
		"reason":"I need L1 access for a real integration.",
		"ai_recommendation":"The user described a concrete integration workflow and can be reviewed for L1 access.",
		"confirmation_token":"`+token+`",
		"confirmed":true
	}`))
	request.Header.Set("Content-Type", "application/json")
	response := httptest.NewRecorder()
	engine.ServeHTTP(response, request)

	assert.Equal(t, http.StatusServiceUnavailable, response.Code)
	assert.Equal(t, "2", response.Header().Get("Retry-After"))
	assert.Contains(t, response.Body.String(), "DEVELOPER_ACCESS_QUEUE_UNAVAILABLE")
	flow, err := model.GetAuthFlow(token, model.AuthFlowMatch{
		Purpose:   model.AuthFlowPurposeAssistantL1,
		UserId:    user.Id,
		SessionId: "developer-access-test-session",
	})
	require.NoError(t, err)
	assert.Zero(t, flow.ConsumedAt)
}
