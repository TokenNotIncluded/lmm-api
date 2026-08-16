package router

import (
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/LIghtJUNction/api.lmm.best/model"
	"github.com/gin-gonic/gin"
	"github.com/glebarez/sqlite"
	"github.com/stretchr/testify/require"
	"gorm.io/gorm"
)

func TestProtectedMutationRoutesRejectOversizedJSONBeforeBinding(t *testing.T) {
	previousDB := model.DB
	previousRedisEnabled := common.RedisEnabled
	previousMainDatabaseType, previousLogDatabaseType := common.MainDatabaseType(), common.LogDatabaseType()
	common.RedisEnabled = false
	common.SetDatabaseTypes(common.DatabaseTypeSQLite, previousLogDatabaseType)
	dsn := fmt.Sprintf("file:%s?mode=memory&cache=shared", strings.ReplaceAll(t.Name(), "/", "_"))
	db, err := gorm.Open(sqlite.Open(dsn), &gorm.Config{})
	require.NoError(t, err)
	model.DB = db
	require.NoError(t, db.AutoMigrate(&model.User{}, &model.Token{}))
	t.Cleanup(func() {
		model.DB = previousDB
		common.RedisEnabled = previousRedisEnabled
		common.SetDatabaseTypes(previousMainDatabaseType, previousLogDatabaseType)
		if sqlDB, sqlErr := db.DB(); sqlErr == nil {
			_ = sqlDB.Close()
		}
	})

	levelOne := model.TrustLevelMinUser + 1
	accessToken := "token-limit-test-pat"
	user := model.User{
		Username:           "token-limit-user",
		Password:           "password-placeholder",
		AffCode:            "token-limit-aff",
		Group:              "default",
		Role:               common.RoleCommonUser,
		Status:             common.UserStatusEnabled,
		AccessToken:        &accessToken,
		TrustLevelOverride: &levelOne,
	}
	require.NoError(t, db.Create(&user).Error)

	gin.SetMode(gin.TestMode)
	engine := gin.New()
	SetApiRouter(engine)
	body := `{"name":"` + strings.Repeat("x", tokenMutationRequestMaxBytes) + `"}`
	request := httptest.NewRequest(http.MethodPost, "/api/token/", strings.NewReader(body))
	request.Header.Set("Authorization", "Bearer "+accessToken)
	response := httptest.NewRecorder()

	engine.ServeHTTP(response, request)

	require.Equal(t, http.StatusRequestEntityTooLarge, response.Code)

	subscriptionBody := `{"plan_id":1,"padding":"` + strings.Repeat("x", subscriptionMutationRequestMaxBytes) + `"}`
	subscriptionRequest := httptest.NewRequest(http.MethodPost, "/api/subscription/balance/pay", strings.NewReader(subscriptionBody))
	subscriptionRequest.Header.Set("Authorization", "Bearer "+accessToken)
	subscriptionResponse := httptest.NewRecorder()

	engine.ServeHTTP(subscriptionResponse, subscriptionRequest)

	require.Equal(t, http.StatusRequestEntityTooLarge, subscriptionResponse.Code)

	topUpBody := `{"amount":1,"padding":"` + strings.Repeat("x", topUpMutationRequestMaxBytes) + `"}`
	topUpRequest := httptest.NewRequest(http.MethodPost, "/api/user/stripe/pay", strings.NewReader(topUpBody))
	topUpRequest.Header.Set("Authorization", "Bearer "+accessToken)
	topUpResponse := httptest.NewRecorder()

	engine.ServeHTTP(topUpResponse, topUpRequest)

	require.Equal(t, http.StatusRequestEntityTooLarge, topUpResponse.Code)

	affTransferBody := `{"aff_code":"` + strings.Repeat("x", topUpMutationRequestMaxBytes) + `"}`
	affTransferRequest := httptest.NewRequest(http.MethodPost, "/api/user/aff_transfer", strings.NewReader(affTransferBody))
	affTransferRequest.Header.Set("Authorization", "Bearer "+accessToken)
	affTransferResponse := httptest.NewRecorder()

	engine.ServeHTTP(affTransferResponse, affTransferRequest)

	require.Equal(t, http.StatusRequestEntityTooLarge, affTransferResponse.Code)
}

func TestEmailBindRouteRejectsOversizedJSONBeforeAuthentication(t *testing.T) {
	gin.SetMode(gin.TestMode)
	engine := gin.New()
	SetApiRouter(engine)
	body := `{"email":"` + strings.Repeat("x", userSelfMutationRequestMaxBytes) + `"}`
	request := httptest.NewRequest(http.MethodPost, "/api/oauth/email/bind", strings.NewReader(body))
	response := httptest.NewRecorder()

	engine.ServeHTTP(response, request)

	require.Equal(t, http.StatusRequestEntityTooLarge, response.Code)
}

func TestAdvancedSecuritySettingsRejectOversizedPolicyBeforeAuthentication(t *testing.T) {
	gin.SetMode(gin.TestMode)
	engine := gin.New()
	SetApiRouter(engine)
	body := `{"rules":"` + strings.Repeat("x", rawOptionMutationRequestMaxBytes) + `"}`
	request := httptest.NewRequest(http.MethodPut, "/api/security/admin/settings", strings.NewReader(body))
	response := httptest.NewRecorder()

	engine.ServeHTTP(response, request)

	require.Equal(t, http.StatusRequestEntityTooLarge, response.Code)
}

func TestCompactOAuthRoutesRejectOversizedJSONBeforeAuthentication(t *testing.T) {
	gin.SetMode(gin.TestMode)
	engine := gin.New()
	SetApiRouter(engine)
	body := `{"provider":"` + strings.Repeat("x", compactOAuthRequestMaxBytes) + `"}`

	for _, path := range []string{"/api/oauth/state", "/api/oauth/wechat/bind", "/api/oauth/telegram/bind/start"} {
		request := httptest.NewRequest(http.MethodPost, path, strings.NewReader(body))
		response := httptest.NewRecorder()

		engine.ServeHTTP(response, request)

		require.Equalf(t, http.StatusRequestEntityTooLarge, response.Code, "path=%s", path)
	}
}
