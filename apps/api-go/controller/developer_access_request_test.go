package controller

import (
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/QuantumNous/new-api/common"
	"github.com/QuantumNous/new-api/model"
	"github.com/gin-gonic/gin"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func setupDeveloperAccessRequestControllerTest(t *testing.T) (*model.User, *gin.Engine) {
	t.Helper()
	gin.SetMode(gin.TestMode)
	db := setupTokenControllerTestDB(t)
	require.NoError(t, db.AutoMigrate(&model.User{}, &model.TopUp{}, &model.DeveloperAccessRequest{}, &model.AuthFlow{}))
	user := &model.User{
		Username: "developer-access-controller-user",
		Password: "password",
		Role:     common.RoleCommonUser,
		Status:   common.UserStatusEnabled,
		Group:    "default",
	}
	require.NoError(t, db.Create(user).Error)
	engine := gin.New()
	engine.POST("/request", func(c *gin.Context) {
		c.Set("id", user.Id)
		c.Set("session_id", "developer-access-test-session")
		SubmitDeveloperAccessRequest(c)
	})
	return user, engine
}

func TestSubmitDeveloperAccessRequestRequiresConfirmedAIRecommendation(t *testing.T) {
	user, engine := setupDeveloperAccessRequestControllerTest(t)

	request := httptest.NewRequest(http.MethodPost, "/request", strings.NewReader(`{
		"reason":"I want to connect Claude Code for a Go project.",
		"ai_recommendation":"The user gave a concrete development use case and compatible client.",
		"confirmed":false
	}`))
	request.Header.Set("Content-Type", "application/json")
	response := httptest.NewRecorder()
	engine.ServeHTTP(response, request)
	assert.Equal(t, http.StatusUnprocessableEntity, response.Code)
	assert.Contains(t, response.Body.String(), "DEVELOPER_ACCESS_CONFIRMATION_REQUIRED")
	stored, err := model.GetDeveloperAccessRequest(user.Id)
	require.NoError(t, err)
	assert.Nil(t, stored)
	request = httptest.NewRequest(http.MethodPost, "/request", strings.NewReader(`{
		"reason":"I want to connect Claude Code for a Go project.",
		"ai_recommendation":"The user gave a concrete development use case and compatible client.",
		"confirmed":true
	}`))
	request.Header.Set("Content-Type", "application/json")
	response = httptest.NewRecorder()
	engine.ServeHTTP(response, request)
	assert.Equal(t, http.StatusOK, response.Code)
	assert.Contains(t, response.Body.String(), model.DeveloperAccessRequestSourceAI)
	stored, err = model.GetDeveloperAccessRequest(user.Id)
	require.NoError(t, err)
	require.NotNil(t, stored)
	assert.Equal(t, "The user gave a concrete development use case and compatible client.", stored.AIRecommendation)

	payload, err := common.Marshal(assistantL1RecommendationDraft{
		UserStatement:  "I want to connect Claude Code for a Go project.",
		Recommendation: "The user gave a concrete development use case and compatible client.",
	})
	require.NoError(t, err)
	confirmationToken, _, err := model.CreateAuthFlow(model.AuthFlowCreate{
		Purpose:   model.AuthFlowPurposeAssistantL1,
		UserId:    user.Id,
		SessionId: "developer-access-test-session",
		Payload:   string(payload),
		ExpiresAt: time.Now().Add(time.Minute),
	})
	require.NoError(t, err)

	request = httptest.NewRequest(http.MethodPost, "/request", strings.NewReader(`{
		"reason":"I want to connect Claude Code for a Go project.",
		"ai_recommendation":"The user gave a concrete development use case and compatible client.",
		"confirmation_token":"`+confirmationToken+`",
		"confirmed":true
	}`))
	request.Header.Set("Content-Type", "application/json")
	response = httptest.NewRecorder()
	engine.ServeHTTP(response, request)
	assert.Equal(t, http.StatusOK, response.Code)
	assert.Contains(t, response.Body.String(), model.DeveloperAccessRequestSourceAI)
	assert.Contains(t, response.Body.String(), "ai_recommendation")
}

func TestSubmitPreparedL1RecommendationCommitsExactlyOnceAfterConfirmation(t *testing.T) {
	user, engine := setupDeveloperAccessRequestControllerTest(t)
	originalRecommendation := "The AI drafted a concrete recommendation for the user's integration workflow."
	editedRecommendation := "The user edited this concrete recommendation before explicitly confirming it."
	payload, err := common.Marshal(assistantL1RecommendationDraft{
		UserStatement:  "I need L1 access for a concrete integration workflow.",
		Recommendation: originalRecommendation,
	})
	require.NoError(t, err)
	confirmationToken, _, err := model.CreateAuthFlow(model.AuthFlowCreate{
		Purpose:   model.AuthFlowPurposeAssistantL1,
		UserId:    user.Id,
		SessionId: "developer-access-test-session",
		Payload:   string(payload),
		ExpiresAt: time.Now().Add(time.Minute),
	})
	require.NoError(t, err)

	stored, err := model.GetDeveloperAccessRequest(user.Id)
	require.NoError(t, err)
	assert.Nil(t, stored)

	body := `{
		"reason":"` + editedRecommendation + `",
		"ai_recommendation":"` + editedRecommendation + `",
		"confirmation_token":"` + confirmationToken + `",
		"confirmed":true
	}`
	request := httptest.NewRequest(http.MethodPost, "/request", strings.NewReader(body))
	request.Header.Set("Content-Type", "application/json")
	response := httptest.NewRecorder()
	engine.ServeHTTP(response, request)
	assert.Equal(t, http.StatusOK, response.Code)

	stored, err = model.GetDeveloperAccessRequest(user.Id)
	require.NoError(t, err)
	require.NotNil(t, stored)
	assert.Equal(t, editedRecommendation, stored.Reason)
	assert.Equal(t, editedRecommendation, stored.AIRecommendation)
	assert.Equal(t, model.DeveloperAccessRequestSourceAI, stored.Source)
	requestID := stored.Id
	_, err = model.GetAuthFlow(confirmationToken, model.AuthFlowMatch{
		Purpose:   model.AuthFlowPurposeAssistantL1,
		UserId:    user.Id,
		SessionId: "developer-access-test-session",
	})
	assert.ErrorIs(t, err, model.ErrAuthFlowConsumed)

	request = httptest.NewRequest(http.MethodPost, "/request", strings.NewReader(body))
	request.Header.Set("Content-Type", "application/json")
	response = httptest.NewRecorder()
	engine.ServeHTTP(response, request)
	assert.Equal(t, http.StatusUnprocessableEntity, response.Code)
	assert.Contains(t, response.Body.String(), "DEVELOPER_ACCESS_AI_CONFIRMATION_INVALID")

	var requestCount int64
	require.NoError(t, model.DB.Model(&model.DeveloperAccessRequest{}).Where("user_id = ?", user.Id).Count(&requestCount).Error)
	assert.EqualValues(t, 1, requestCount)
	stored, err = model.GetDeveloperAccessRequest(user.Id)
	require.NoError(t, err)
	require.NotNil(t, stored)
	assert.Equal(t, requestID, stored.Id)
	assert.Equal(t, editedRecommendation, stored.AIRecommendation)
}
