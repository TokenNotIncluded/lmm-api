package controller

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/LIghtJUNction/api.lmm.best/model"
	"github.com/gin-gonic/gin"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestL1OnboardingControllerDoesNotLetBrowserForgeCompletion(t *testing.T) {
	db := setupUserOnboardingTestDB(t)
	require.NoError(t, db.AutoMigrate(&model.L1OnboardingTodo{}))
	levelOne := model.TrustLevelMinUser + 1
	user := model.User{
		Username:           "l1-controller-onboarding",
		Password:           "password",
		Role:               common.RoleCommonUser,
		Status:             common.UserStatusEnabled,
		TrustLevelOverride: &levelOne,
	}
	require.NoError(t, db.Create(&user).Error)

	gin.SetMode(gin.TestMode)
	recorder := httptest.NewRecorder()
	c, _ := gin.CreateTestContext(recorder)
	c.Set("id", user.Id)
	c.Request = httptest.NewRequest(http.MethodPatch, "/api/user/self/onboarding/todo", strings.NewReader(`{"completed":true,"step":"first_successful_response"}`))
	c.Request.Header.Set("Content-Type", "application/json")
	PatchL1OnboardingTodo(c)

	assert.Equal(t, http.StatusOK, recorder.Code)
	var response struct {
		Success bool                       `json:"success"`
		Data    model.L1OnboardingTodoView `json:"data"`
	}
	require.NoError(t, json.Unmarshal(recorder.Body.Bytes(), &response))
	assert.True(t, response.Success)
	assert.Equal(t, model.L1OnboardingStepCreateAPIKey, response.Data.CurrentStep)
	assert.Equal(t, model.L1OnboardingStatusInProgress, response.Data.Status)
	assert.Zero(t, response.Data.CompletedAt)

	var todo model.L1OnboardingTodo
	require.NoError(t, db.Where("user_id = ?", user.Id).First(&todo).Error)
	assert.Zero(t, todo.CompletedAt)

	var persisted model.User
	require.NoError(t, db.First(&persisted, user.Id).Error)
	require.NotNil(t, persisted.TrustLevelOverride)
	assert.Equal(t, levelOne, *persisted.TrustLevelOverride)
}

func TestL1OnboardingControllerReturnsUnavailableForL0(t *testing.T) {
	db := setupUserOnboardingTestDB(t)
	require.NoError(t, db.AutoMigrate(&model.L1OnboardingTodo{}))
	user := model.User{Username: "l0-controller-onboarding", Password: "password", Role: common.RoleCommonUser, Status: common.UserStatusEnabled}
	require.NoError(t, db.Create(&user).Error)

	gin.SetMode(gin.TestMode)
	recorder := httptest.NewRecorder()
	c, _ := gin.CreateTestContext(recorder)
	c.Set("id", user.Id)
	c.Request = httptest.NewRequest(http.MethodGet, "/api/user/self/onboarding/todo", nil)
	GetL1OnboardingTodo(c)

	assert.Equal(t, http.StatusOK, recorder.Code)
	var response struct {
		Success bool                       `json:"success"`
		Data    model.L1OnboardingTodoView `json:"data"`
	}
	require.NoError(t, json.Unmarshal(recorder.Body.Bytes(), &response))
	assert.True(t, response.Success)
	assert.False(t, response.Data.Eligibility.Eligible)
	assert.Equal(t, "unavailable", response.Data.Status)
	assert.Empty(t, response.Data.Steps)

	var count int64
	require.NoError(t, db.Model(&model.L1OnboardingTodo{}).Where("user_id = ?", user.Id).Count(&count).Error)
	assert.Zero(t, count)
}
