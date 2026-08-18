package controller

import (
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/LIghtJUNction/api.lmm.best/i18n"
	"github.com/LIghtJUNction/api.lmm.best/model"
	"github.com/LIghtJUNction/api.lmm.best/service"
	"github.com/gin-gonic/gin"
	"github.com/glebarez/sqlite"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
	"gorm.io/gorm"
)

func TestWeChatLoginStartPersistsLegalConsentInBrowserBoundFlow(t *testing.T) {
	setupAuthFlowControllerTest(t)
	previousEnabled := common.WeChatAuthEnabled
	common.WeChatAuthEnabled = true
	t.Cleanup(func() { common.WeChatAuthEnabled = previousEnabled })

	recorder := httptest.NewRecorder()
	c, _ := gin.CreateTestContext(recorder)
	c.Request = httptest.NewRequest(http.MethodPost, "/api/oauth/wechat/start", strings.NewReader(`{"accepted_legal":true}`))
	c.Request.Header.Set("Content-Type", "application/json")

	WeChatAuthStart(c)

	assert.Equal(t, http.StatusOK, recorder.Code)
	var response struct {
		Success bool `json:"success"`
		Data    struct {
			FlowToken string `json:"flow_token"`
		} `json:"data"`
	}
	require.NoError(t, common.Unmarshal(recorder.Body.Bytes(), &response))
	require.True(t, response.Success)
	require.NotEmpty(t, response.Data.FlowToken)
	setCookie := recorder.Header().Get("Set-Cookie")
	assert.Contains(t, setCookie, oauthStateCookieName("wechat")+"="+response.Data.FlowToken)
	assert.Contains(t, setCookie, "; Secure")

	flow, err := model.GetAuthFlow(response.Data.FlowToken, model.AuthFlowMatch{
		Purpose: model.AuthFlowPurposeWeChatLogin, Provider: "wechat", Intent: model.AuthFlowIntentLogin,
	})
	require.NoError(t, err)
	var payload oauthFlowPayload
	require.NoError(t, common.UnmarshalJsonStr(flow.Payload, &payload))
	assert.True(t, payload.AcceptedLegal)
}

func TestWeChatLoginRejectsCallbackWithoutBrowserState(t *testing.T) {
	setupAuthFlowControllerTest(t)
	previousEnabled := common.WeChatAuthEnabled
	common.WeChatAuthEnabled = true
	t.Cleanup(func() { common.WeChatAuthEnabled = previousEnabled })

	router := gin.New()
	router.GET("/api/oauth/wechat", WeChatAuth)
	request := httptest.NewRequest(http.MethodGet, "/api/oauth/wechat?code=provider-code&state=unbound-state", nil)
	recorder := httptest.NewRecorder()
	router.ServeHTTP(recorder, request)

	assert.Equal(t, http.StatusForbidden, recorder.Code)
	assert.NotContains(t, recorder.Body.String(), "验证码错误或已过期")
}

func TestWeChatProviderErrorDoesNotEchoUpstreamMessage(t *testing.T) {
	setupAuthFlowControllerTest(t)
	previousEnabled := common.WeChatAuthEnabled
	previousAddress := common.WeChatServerAddress
	previousToken := common.WeChatServerToken
	previousValidator := validateWeChatProviderURL
	previousClientFactory := newWeChatHTTPClient
	common.WeChatAuthEnabled = true
	providerMessage := "upstream database password leaked"
	provider := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, _ *http.Request) {
		writer.Header().Set("Content-Type", "application/json")
		_, _ = writer.Write([]byte(`{"success":false,"message":"` + providerMessage + `"}`))
	}))
	common.WeChatServerAddress = provider.URL
	common.WeChatServerToken = "wechat-test-token"
	// The production validator rejects loopback addresses; this test server is
	// intentionally local and exercises the response-redaction contract.
	validateWeChatProviderURL = func(string) error { return nil }
	newWeChatHTTPClient = func() *http.Client { return &http.Client{Timeout: 5 * time.Second} }
	t.Cleanup(func() {
		provider.Close()
		common.WeChatAuthEnabled = previousEnabled
		common.WeChatServerAddress = previousAddress
		common.WeChatServerToken = previousToken
		validateWeChatProviderURL = previousValidator
		newWeChatHTTPClient = previousClientFactory
	})

	state, _, err := model.CreateAuthFlow(model.AuthFlowCreate{
		Purpose: model.AuthFlowPurposeWeChatLogin, Provider: "wechat", Intent: model.AuthFlowIntentLogin,
		ExpiresAt: time.Now().Add(time.Minute),
	})
	require.NoError(t, err)
	router := gin.New()
	router.GET("/api/oauth/wechat", WeChatAuth)
	request := httptest.NewRequest(http.MethodGet, "/api/oauth/wechat?code=provider-code&state="+state, nil)
	request.AddCookie(&http.Cookie{Name: oauthStateCookieName("wechat"), Value: state})
	recorder := httptest.NewRecorder()
	router.ServeHTTP(recorder, request)

	assert.Equal(t, http.StatusOK, recorder.Code)
	assert.NotContains(t, recorder.Body.String(), providerMessage)
	var response struct {
		Message string `json:"message"`
	}
	require.NoError(t, common.Unmarshal(recorder.Body.Bytes(), &response))
	assert.NotEmpty(t, response.Message)
	assert.NotEqual(t, providerMessage, response.Message)
}

func TestWeChatProviderResponseIsBounded(t *testing.T) {
	previousAddress := common.WeChatServerAddress
	previousToken := common.WeChatServerToken
	previousValidator := validateWeChatProviderURL
	previousClientFactory := newWeChatHTTPClient
	provider := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, _ *http.Request) {
		writer.Header().Set("Content-Type", "application/json")
		_, _ = writer.Write([]byte(`{"success":false,"message":"` + strings.Repeat("x", int(wechatProviderResponseMaxBytes)) + `"}`))
	}))
	common.WeChatServerAddress = provider.URL
	common.WeChatServerToken = "wechat-test-token"
	validateWeChatProviderURL = func(string) error { return nil }
	newWeChatHTTPClient = func() *http.Client { return &http.Client{Timeout: 5 * time.Second} }
	t.Cleanup(func() {
		provider.Close()
		common.WeChatServerAddress = previousAddress
		common.WeChatServerToken = previousToken
		validateWeChatProviderURL = previousValidator
		newWeChatHTTPClient = previousClientFactory
	})

	_, err := getWeChatIdByCode("provider-code")
	assert.ErrorIs(t, err, common.ErrLimitExceeded)
}

func TestValidateWeChatProviderURLRejectsPrivateAndMalformedTargets(t *testing.T) {
	for _, rawURL := range []string{
		"http://127.0.0.1:8080",
		"http://169.254.169.254/latest/meta-data",
		"file:///etc/passwd",
		"https://user:password@example.com",
	} {
		t.Run(rawURL, func(t *testing.T) {
			assert.Error(t, validateWeChatProviderURL(rawURL))
		})
	}
}

func TestTelegramLoginStartPersistsBrowserBoundFlow(t *testing.T) {
	setupAuthFlowControllerTest(t)
	previousEnabled := common.TelegramOAuthEnabled
	common.TelegramOAuthEnabled = true
	t.Cleanup(func() { common.TelegramOAuthEnabled = previousEnabled })

	recorder := httptest.NewRecorder()
	c, _ := gin.CreateTestContext(recorder)
	c.Request = httptest.NewRequest(http.MethodPost, "/api/oauth/telegram/login/start", nil)

	TelegramLoginStart(c)

	assert.Equal(t, http.StatusOK, recorder.Code)
	var response struct {
		Success bool `json:"success"`
		Data    struct {
			FlowToken string `json:"flow_token"`
		} `json:"data"`
	}
	require.NoError(t, common.Unmarshal(recorder.Body.Bytes(), &response))
	require.True(t, response.Success)
	require.NotEmpty(t, response.Data.FlowToken)
	assert.Contains(t, recorder.Header().Get("Set-Cookie"), oauthStateCookieName("telegram")+"="+response.Data.FlowToken)
	_, err := model.GetAuthFlow(response.Data.FlowToken, model.AuthFlowMatch{
		Purpose: model.AuthFlowPurposeTelegramLogin, Provider: "telegram", Intent: model.AuthFlowIntentLogin,
	})
	require.NoError(t, err)
}

func TestOAuthBindRejectsLeakedStateWithoutBrowserCookie(t *testing.T) {
	setupAuthFlowControllerTest(t)
	user := &model.User{
		Username: "oauth-bind-state-user", Password: "password-placeholder", Role: common.RoleCommonUser,
		Status: common.UserStatusEnabled, Group: "default", AuthVersion: 1,
	}
	require.NoError(t, model.DB.Create(user).Error)
	bundle, err := service.CreateLoginSession(user.Id, "password", "127.0.0.1", "oauth-bind-state-test")
	require.NoError(t, err)
	state, _, err := model.CreateAuthFlow(model.AuthFlowCreate{
		Purpose: model.AuthFlowPurposeOAuth, Provider: "auth-flow-test", Intent: model.AuthFlowIntentBind,
		UserId: user.Id, SessionId: bundle.Session.SID, ExpiresAt: time.Now().Add(time.Minute),
	})
	require.NoError(t, err)

	router := gin.New()
	router.GET("/api/oauth/:provider", HandleOAuth)
	request := httptest.NewRequest(http.MethodGet, "/api/oauth/auth-flow-test?state="+state+"&error=access_denied", nil)
	recorder := httptest.NewRecorder()
	router.ServeHTTP(recorder, request)

	assert.Equal(t, http.StatusForbidden, recorder.Code)
	_, err = model.GetAuthFlow(state, model.AuthFlowMatch{Purpose: model.AuthFlowPurposeOAuth})
	assert.NoError(t, err)
}

func TestTelegramLoginRejectsCallbackWithoutBrowserState(t *testing.T) {
	setupAuthFlowControllerTest(t)
	previousEnabled := common.TelegramOAuthEnabled
	previousToken := common.TelegramBotToken
	common.TelegramOAuthEnabled = true
	common.TelegramBotToken = "telegram-state-test-token"
	t.Cleanup(func() {
		common.TelegramOAuthEnabled = previousEnabled
		common.TelegramBotToken = previousToken
	})

	params := signedTelegramAuthorization(common.TelegramBotToken, time.Now())
	params.Set("state", "unbound-state")
	router := gin.New()
	router.GET("/api/oauth/telegram/login", TelegramLogin)
	request := httptest.NewRequest(http.MethodGet, "/api/oauth/telegram/login?"+params.Encode(), nil)
	recorder := httptest.NewRecorder()
	router.ServeHTTP(recorder, request)

	assert.Equal(t, http.StatusForbidden, recorder.Code)
	assert.Contains(t, recorder.Body.String(), "无效的登录状态")
}

func TestTelegramLoginAcceptsProviderSignatureWithoutSigningApplicationState(t *testing.T) {
	previousDB, previousLogDB := model.DB, model.LOG_DB
	previousType := common.MainDatabaseType()
	previousRedis := common.RedisEnabled
	previousEnabled := common.TelegramOAuthEnabled
	previousToken := common.TelegramBotToken
	previousSecret := common.SessionSecret
	db, err := gorm.Open(sqlite.Open(":memory:"), &gorm.Config{})
	require.NoError(t, err)
	require.NoError(t, db.AutoMigrate(&model.AuthFlow{}, &model.User{}, &model.UserSession{}, &model.Log{}))
	model.DB, model.LOG_DB = db, db
	common.SetMainDatabaseType(common.DatabaseTypeSQLite)
	common.RedisEnabled = false
	common.TelegramOAuthEnabled = true
	common.TelegramBotToken = "telegram-state-test-token"
	common.SessionSecret = "telegram-state-test-secret"
	t.Cleanup(func() {
		model.DB, model.LOG_DB = previousDB, previousLogDB
		common.SetMainDatabaseType(previousType)
		common.RedisEnabled = previousRedis
		common.TelegramOAuthEnabled = previousEnabled
		common.TelegramBotToken = previousToken
		common.SessionSecret = previousSecret
	})
	require.NoError(t, i18n.Init())

	user := &model.User{
		Username: "telegram-state-user", Password: "password-placeholder", TelegramId: "123456",
		Role: common.RoleCommonUser, Status: common.UserStatusEnabled, Group: "default", AuthVersion: 1,
		AffCode: "telegram-state-user",
	}
	require.NoError(t, db.Create(user).Error)
	state, _, err := model.CreateAuthFlow(model.AuthFlowCreate{
		Purpose: model.AuthFlowPurposeTelegramLogin, Provider: "telegram", Intent: model.AuthFlowIntentLogin,
		ExpiresAt: time.Now().Add(time.Minute),
	})
	require.NoError(t, err)

	params := signedTelegramAuthorization(common.TelegramBotToken, time.Now())
	params.Set("state", state)
	router := gin.New()
	router.GET("/api/oauth/telegram/login", TelegramLogin)
	request := httptest.NewRequest(http.MethodGet, "/api/oauth/telegram/login?"+params.Encode(), nil)
	request.AddCookie(&http.Cookie{Name: oauthStateCookieName("telegram"), Value: state})
	recorder := httptest.NewRecorder()
	router.ServeHTTP(recorder, request)

	assert.Equal(t, http.StatusOK, recorder.Code)
	assert.Contains(t, recorder.Body.String(), `"success":true`)
	_, err = model.GetAuthFlow(state, model.AuthFlowMatch{Purpose: model.AuthFlowPurposeTelegramLogin})
	assert.ErrorIs(t, err, model.ErrAuthFlowConsumed)
	var assertionCount int64
	require.NoError(t, db.Model(&model.AuthFlow{}).Where("purpose = ?", model.AuthFlowPurposeTelegramAssertion).Count(&assertionCount).Error)
	assert.Equal(t, int64(1), assertionCount)
}
