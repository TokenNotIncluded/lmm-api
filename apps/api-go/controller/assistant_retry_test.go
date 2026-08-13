/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program. If not, see <https://www.gnu.org/licenses/>.

For commercial licensing, please contact support@quantumnous.com
*/
package controller

import (
	"errors"
	"net/http"
	"net/http/httptest"
	"strconv"
	"strings"
	"testing"

	"github.com/QuantumNous/new-api/common"
	"github.com/QuantumNous/new-api/model"
	"github.com/QuantumNous/new-api/relaykit/types"
	"github.com/QuantumNous/new-api/service"
	"github.com/QuantumNous/new-api/setting"
	"github.com/gin-gonic/gin"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestAssistantRelayRetryPolicyClassifiesTransientHTTPAndNetworkErrors(t *testing.T) {
	gin.SetMode(gin.TestMode)

	transientStatuses := []int{
		http.StatusTooManyRequests,
		http.StatusInternalServerError,
		http.StatusBadGateway,
		http.StatusServiceUnavailable,
		http.StatusNetworkAuthenticationRequired,
	}
	for _, status := range transientStatuses {
		t.Run("http-"+strconv.Itoa(status), func(t *testing.T) {
			c, _ := gin.CreateTestContext(httptest.NewRecorder())
			apiErr := types.NewOpenAIError(
				errors.New("transient assistant upstream failure"),
				types.ErrorCodeBadResponseStatusCode,
				status,
			)

			assert.True(t, service.ShouldRetryRelayError(c, apiErr, 1))
		})
	}

	networkErr := types.NewErrorWithStatusCode(
		errors.New("dial tcp: connection reset by peer"),
		types.ErrorCodeDoRequestFailed,
		0,
	)
	c, _ := gin.CreateTestContext(httptest.NewRecorder())
	assert.True(t, service.ShouldRetryRelayError(c, networkErr, 1))

	for _, status := range []int{
		http.StatusBadRequest,
		http.StatusRequestTimeout,
		http.StatusGatewayTimeout,
		524,
	} {
		t.Run("hard-stop-http-"+strconv.Itoa(status), func(t *testing.T) {
			c, _ := gin.CreateTestContext(httptest.NewRecorder())
			apiErr := types.NewOpenAIError(
				errors.New("assistant upstream failure must not be retried"),
				types.ErrorCodeBadResponseStatusCode,
				status,
			)

			assert.False(t, service.ShouldRetryRelayError(c, apiErr, 1))
		})
	}
}

func TestAssistantRelayRetryPolicyStopsWhenAttemptBudgetIsExhausted(t *testing.T) {
	gin.SetMode(gin.TestMode)
	c, _ := gin.CreateTestContext(httptest.NewRecorder())
	apiErr := types.NewOpenAIError(
		errors.New("assistant upstream unavailable"),
		types.ErrorCodeBadResponseStatusCode,
		http.StatusServiceUnavailable,
	)

	assert.True(t, service.ShouldRetryRelayError(c, apiErr, 1))
	assert.False(t, service.ShouldRetryRelayError(c, apiErr, 0))
}

func TestAssistantRetryRejectsInvalidInputBeforePersistentWrites(t *testing.T) {
	gin.SetMode(gin.TestMode)
	db := setupTokenControllerTestDB(t)
	require.NoError(t, db.AutoMigrate(
		&model.User{},
		&model.UserOAuthBinding{},
		&model.AssistantUserProfile{},
		&model.TopUp{},
		&model.AssistantLead{},
		&model.AssistantProfileBucket{},
		&model.AssistantFirstQuestionStat{},
		&model.DeveloperAccessRequest{},
	))
	withAssistantSettings(t, true, "assistant-retry-validation-model")

	user := model.User{
		Username:           "assistant-retry-validation-user",
		Password:           "password",
		Role:               common.RoleCommonUser,
		Status:             common.UserStatusEnabled,
		Group:              "default",
		ConsoleActivatedAt: 1,
	}
	require.NoError(t, db.Create(&user).Error)

	engine := gin.New()
	engine.POST("/api/assistant/chat", func(c *gin.Context) {
		c.Set("id", user.Id)
		PrepareAssistantRequest(c)
	})

	request := httptest.NewRequest(
		http.MethodPost,
		"/api/assistant/chat",
		strings.NewReader(`{"message":"."}`),
	)
	request.Header.Set("Content-Type", "application/json")
	request.Header.Set("X-LMM-Assistant-Attempt", "1")
	response := httptest.NewRecorder()
	engine.ServeHTTP(response, request)

	assert.Equal(t, http.StatusBadRequest, response.Code)
	assert.Contains(t, response.Body.String(), "ASSISTANT_SINGLE_PUNCTUATION")

	var conversationCount int64
	var intentCount int64
	var firstQuestionCount int64
	var developerAccessCount int64
	require.NoError(t, db.Model(&model.AssistantConversation{}).Count(&conversationCount).Error)
	require.NoError(t, db.Model(&model.AssistantLead{}).Count(&intentCount).Error)
	require.NoError(t, db.Model(&model.AssistantFirstQuestionStat{}).Count(&firstQuestionCount).Error)
	require.NoError(t, db.Model(&model.DeveloperAccessRequest{}).Count(&developerAccessCount).Error)
	assert.Zero(t, conversationCount)
	assert.Zero(t, intentCount)
	assert.Zero(t, firstQuestionCount)
	assert.Zero(t, developerAccessCount)
}

func TestAssistantRetryKeepsL1QueueWriteIdempotent(t *testing.T) {
	gin.SetMode(gin.TestMode)
	db := setupTokenControllerTestDB(t)
	require.NoError(t, db.AutoMigrate(
		&model.User{},
		&model.UserOAuthBinding{},
		&model.AssistantUserProfile{},
		&model.TopUp{},
		&model.AssistantLead{},
		&model.AssistantProfileBucket{},
		&model.AssistantFirstQuestionStat{},
		&model.DeveloperAccessRequest{},
	))
	withAssistantSettings(t, true, "assistant-retry-idempotency-model")
	originalSettings := setting.GetAssistantSettings()
	setting.SetAssistantCacheEnabled(false)
	t.Cleanup(func() { setting.SetAssistantCacheEnabled(originalSettings.CacheEnabled) })

	user := model.User{
		Username: "assistant-retry-l0-user",
		Password: "password",
		Role:     common.RoleCommonUser,
		Status:   common.UserStatusEnabled,
		Group:    "default",
	}
	require.NoError(t, db.Create(&user).Error)

	engine := gin.New()
	engine.POST("/api/assistant/chat", func(c *gin.Context) {
		c.Set("id", user.Id)
		PrepareAssistantRequest(c)
	}, func(c *gin.Context) {
		c.Status(http.StatusNoContent)
	})

	message := "Please request L1 developer access for my integration"
	for attempt := 1; attempt <= 2; attempt++ {
		request := httptest.NewRequest(
			http.MethodPost,
			"/api/assistant/chat",
			strings.NewReader(`{"message":"`+message+`"}`),
		)
		request.Header.Set("Content-Type", "application/json")
		request.Header.Set("X-LMM-Assistant-Attempt", strconv.Itoa(attempt))
		response := httptest.NewRecorder()
		engine.ServeHTTP(response, request)
		assert.Equal(t, http.StatusNoContent, response.Code)
	}

	var requests []model.DeveloperAccessRequest
	require.NoError(t, db.Where("user_id = ?", user.Id).Find(&requests).Error)
	require.Len(t, requests, 1)
	assert.Equal(t, model.DeveloperAccessRequestPending, requests[0].Status)
	assert.Equal(t, model.DeveloperAccessRequestSourceAssistant, requests[0].Source)
	assert.Equal(t, message, requests[0].Reason)
}

func TestAssistantChatQueuesL1BeforeDownstreamFailure(t *testing.T) {
	gin.SetMode(gin.TestMode)
	db := setupTokenControllerTestDB(t)
	require.NoError(t, db.AutoMigrate(
		&model.User{},
		&model.UserOAuthBinding{},
		&model.AssistantUserProfile{},
		&model.TopUp{},
		&model.AssistantLead{},
		&model.AssistantProfileBucket{},
		&model.AssistantFirstQuestionStat{},
		&model.DeveloperAccessRequest{},
	))
	withAssistantSettings(t, true, "assistant-l1-downstream-failure-model")
	originalSettings := setting.GetAssistantSettings()
	setting.SetAssistantCacheEnabled(false)
	t.Cleanup(func() { setting.SetAssistantCacheEnabled(originalSettings.CacheEnabled) })

	user := model.User{
		Username: "assistant-l1-downstream-failure-user",
		Password: "password",
		Role:     common.RoleCommonUser,
		Status:   common.UserStatusEnabled,
		Group:    "default",
	}
	require.NoError(t, db.Create(&user).Error)

	engine := gin.New()
	engine.POST("/api/assistant/chat", func(c *gin.Context) {
		c.Set("id", user.Id)
		PrepareAssistantRequest(c)
	}, func(c *gin.Context) {
		// The model/provider failed after request preparation. The durable
		// review item must already exist at this point.
		c.AbortWithStatus(http.StatusBadGateway)
	})

	request := httptest.NewRequest(
		http.MethodPost,
		"/api/assistant/chat",
		strings.NewReader(`{"message":"Please request L1 developer access for my integration"}`),
	)
	request.Header.Set("Content-Type", "application/json")
	response := httptest.NewRecorder()
	engine.ServeHTTP(response, request)

	assert.Equal(t, http.StatusBadGateway, response.Code)
	queued, err := model.GetDeveloperAccessRequest(user.Id)
	require.NoError(t, err)
	require.NotNil(t, queued)
	assert.Equal(t, model.DeveloperAccessRequestPending, queued.Status)
	assert.Equal(t, model.DeveloperAccessRequestSourceAssistant, queued.Source)
}

func TestAssistantL1QueueFailureIsRetryableAndStopsDownstream(t *testing.T) {
	gin.SetMode(gin.TestMode)
	db := setupTokenControllerTestDB(t)
	require.NoError(t, db.AutoMigrate(
		&model.User{},
		&model.UserOAuthBinding{},
		&model.AssistantUserProfile{},
		&model.TopUp{},
		&model.AssistantLead{},
		&model.AssistantProfileBucket{},
		&model.AssistantFirstQuestionStat{},
		&model.DeveloperAccessRequest{},
	))
	withAssistantSettings(t, true, "assistant-l1-queue-failure-model")
	originalSettings := setting.GetAssistantSettings()
	setting.SetAssistantCacheEnabled(false)
	t.Cleanup(func() { setting.SetAssistantCacheEnabled(originalSettings.CacheEnabled) })

	user := model.User{
		Username: "assistant-l1-queue-failure-user",
		Password: "password",
		Role:     common.RoleCommonUser,
		Status:   common.UserStatusEnabled,
		Group:    "default",
	}
	require.NoError(t, db.Create(&user).Error)
	require.NoError(t, db.Migrator().DropTable(&model.DeveloperAccessRequest{}))

	downstreamCalls := 0
	engine := gin.New()
	engine.POST("/api/assistant/chat", func(c *gin.Context) {
		c.Set("id", user.Id)
		PrepareAssistantRequest(c)
	}, func(c *gin.Context) {
		downstreamCalls++
		c.Status(http.StatusNoContent)
	})

	request := httptest.NewRequest(
		http.MethodPost,
		"/api/assistant/chat",
		strings.NewReader(`{"message":"Please request L1 developer access for my integration"}`),
	)
	request.Header.Set("Content-Type", "application/json")
	response := httptest.NewRecorder()
	engine.ServeHTTP(response, request)

	assert.Equal(t, http.StatusServiceUnavailable, response.Code)
	assert.Equal(t, "2", response.Header().Get("Retry-After"))
	assert.Contains(t, response.Body.String(), "ASSISTANT_L1_QUEUE_UNAVAILABLE")
	assert.Zero(t, downstreamCalls)
}

func TestAssistantL1RecommendationQueueFailureIsExplicitlyRetryable(t *testing.T) {
	gin.SetMode(gin.TestMode)
	db := setupTokenControllerTestDB(t)
	require.NoError(t, db.AutoMigrate(
		&model.User{},
		&model.TopUp{},
		&model.DeveloperAccessRequest{},
		&model.AuthFlow{},
	))
	withAssistantSettings(t, true, "assistant-l1-recommendation-queue-failure-model")

	levelZero := model.TrustLevelMinUser
	user := model.User{
		Username:           "assistant-l1-recommendation-queue-failure-user",
		Password:           "password",
		Role:               common.RoleCommonUser,
		Status:             common.UserStatusEnabled,
		Group:              "default",
		TrustLevelOverride: &levelZero,
	}
	require.NoError(t, db.Create(&user).Error)
	require.NoError(t, db.Migrator().DropTable(&model.DeveloperAccessRequest{}))

	recorder := httptest.NewRecorder()
	c, _ := gin.CreateTestContext(recorder)
	c.Set("id", user.Id)
	c.Set(assistantActorUserIDKey, user.Id)
	c.Set("session_id", "assistant-l1-queue-failure-session")

	result := executeAssistantL1RecommendationTool(c, user.Id, map[string]any{
		"user_statement": "I need L1 access for my integration.",
		"recommendation": "The user described a concrete integration workflow and can be reviewed for L1 access.",
	})

	assert.Equal(t, false, result["ok"])
	assert.Equal(t, "queue_unavailable", result["status"])
	assert.Equal(t, "ASSISTANT_L1_QUEUE_UNAVAILABLE", result["code"])
	assert.Equal(t, true, result["retryable"])
	assert.Equal(t, http.StatusServiceUnavailable, recorder.Code)
	assert.Equal(t, "2", recorder.Header().Get("Retry-After"))
	assert.Contains(t, recorder.Body.String(), "ASSISTANT_L1_QUEUE_UNAVAILABLE")

	var flowCount int64
	require.NoError(t, db.Model(&model.AuthFlow{}).Count(&flowCount).Error)
	assert.Zero(t, flowCount)
}

func TestAssistantRetryDoesNotDuplicateFirstTurnConversationOnReplay(t *testing.T) {
	gin.SetMode(gin.TestMode)
	db := setupTokenControllerTestDB(t)
	require.NoError(t, db.AutoMigrate(
		&model.AssistantLead{},
		&model.AssistantProfileBucket{},
		&model.AssistantFirstQuestionStat{},
		&model.User{},
		&model.UserOAuthBinding{},
		&model.AssistantUserProfile{},
		&model.TopUp{},
		&model.DeveloperAccessRequest{},
	))
	withAssistantSettings(t, true, "assistant-retry-replay-model")
	originalSettings := setting.GetAssistantSettings()
	setting.SetAssistantCacheEnabled(true)
	require.NoError(t, setting.UpdateAssistantCacheTTLMinutes("10"))
	t.Cleanup(func() {
		setting.SetAssistantCacheEnabled(originalSettings.CacheEnabled)
		_ = setting.UpdateAssistantCacheTTLMinutes(strconv.Itoa(originalSettings.CacheTTLMinutes))
	})

	user := model.User{
		Username: "assistant-retry-replay-user",
		Password: "password",
		Role:     common.RoleCommonUser,
		Status:   common.UserStatusEnabled,
		Group:    "default",
	}
	require.NoError(t, db.Create(&user).Error)

	message := "replay this assistant answer without creating another conversation"
	settings := setting.GetAssistantSettings()
	userContext := assistantUserContextForRequest(user.Id, message)
	cacheKey := assistantCacheKey(
		settings,
		[]assistantOpenAIMessage{{Role: "user", Content: message}},
		userContext,
	)
	require.NotEmpty(t, cacheKey)
	storeAssistantCachedResponse(
		settings,
		cacheKey,
		http.StatusOK,
		[]byte(`{"choices":[{"message":{"role":"assistant","content":"cached answer"}}]}`),
	)

	engine := gin.New()
	engine.POST("/api/assistant/chat", func(c *gin.Context) {
		c.Set("id", user.Id)
		PrepareAssistantRequest(c)
	})

	// A lost successful response can make the browser replay the same first
	// turn after attempt 1 has already populated the response cache. The
	// replay must attach to the original conversation rather than append a
	// second user/assistant pair.
	for attempt := 1; attempt <= 2; attempt++ {
		request := httptest.NewRequest(
			http.MethodPost,
			"/api/assistant/chat",
			strings.NewReader(`{"message":"`+message+`"}`),
		)
		request.Header.Set("Content-Type", "application/json")
		request.Header.Set("X-LMM-Assistant-Attempt", strconv.Itoa(attempt))
		response := httptest.NewRecorder()
		engine.ServeHTTP(response, request)
		assert.Equal(t, http.StatusOK, response.Code)
		assert.Equal(t, "HIT", response.Header().Get("X-LMM-Assistant-Cache"))
	}

	var conversationCount int64
	var historyMessageCount int64
	require.NoError(t, db.Model(&model.AssistantConversation{}).Count(&conversationCount).Error)
	require.NoError(t, db.Model(&model.AssistantHistoryMessage{}).Count(&historyMessageCount).Error)
	assert.EqualValues(t, 1, conversationCount)
	assert.EqualValues(t, 2, historyMessageCount)
}
