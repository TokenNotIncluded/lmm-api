package controller

import (
	"bytes"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"testing"
	"time"

	"github.com/LIghtJUNction/api.lmm.best/model"
	"github.com/LIghtJUNction/api.lmm.best/setting"
	"github.com/LIghtJUNction/api.lmm.best/setting/operation_setting"
	"github.com/gin-gonic/gin"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func preservePaymentGatewaySettings(t *testing.T) {
	t.Helper()
	originalFastPayAddress := setting.FastPayAddress
	originalFastPayMerchantNo := setting.FastPayMerchantNo
	originalFastPayShopNo := setting.FastPayShopNo
	originalFastPayAPISecret := setting.FastPayApiSecret
	originalPayAddress := operation_setting.PayAddress
	originalEpayID := operation_setting.EpayId
	originalEpayKey := operation_setting.EpayKey
	originalPayMethods := operation_setting.PayMethods
	t.Cleanup(func() {
		setting.FastPayAddress = originalFastPayAddress
		setting.FastPayMerchantNo = originalFastPayMerchantNo
		setting.FastPayShopNo = originalFastPayShopNo
		setting.FastPayApiSecret = originalFastPayAPISecret
		operation_setting.PayAddress = originalPayAddress
		operation_setting.EpayId = originalEpayID
		operation_setting.EpayKey = originalEpayKey
		operation_setting.PayMethods = originalPayMethods
	})
}

func TestResolveFastPayMethod_WhenOnlyFastPayIsConfigured(t *testing.T) {
	preservePaymentGatewaySettings(t)
	setting.FastPayAddress = "https://fastpay.example.com/fastpay-server"
	setting.FastPayMerchantNo = "M123"
	setting.FastPayShopNo = "S123"
	setting.FastPayApiSecret = "fastpay_secret"
	operation_setting.PayAddress = "https://pay.example.com"
	operation_setting.EpayId = "10001"
	operation_setting.EpayKey = ""

	method, ok := resolveFastPayMethod("alipay")
	require.True(t, ok)
	require.Equal(t, "alipay", method)
}

func TestResolveFastPayMethod_PrefixedMethodWinsWhenBothGatewaysAreConfigured(t *testing.T) {
	preservePaymentGatewaySettings(t)
	setting.FastPayAddress = "https://fastpay.example.com/fastpay-server"
	setting.FastPayMerchantNo = "M123"
	setting.FastPayShopNo = "S123"
	setting.FastPayApiSecret = "fastpay_secret"
	operation_setting.PayAddress = "https://epay.example.com"
	operation_setting.EpayId = "10001"
	operation_setting.EpayKey = "epay_secret"

	method, ok := resolveFastPayMethod("fastpay_wxpay")
	require.True(t, ok)
	require.Equal(t, "wxpay", method)

	method, ok = resolveFastPayMethod("alipay")
	require.False(t, ok)
	require.Equal(t, "alipay", method)
}

func TestRequestEpayDelegatesFastPayMethodsBeforeEpayPricing(t *testing.T) {
	gin.SetMode(gin.TestMode)
	confirmPaymentComplianceForTest(t)
	preservePaymentGatewaySettings(t)
	preserveChannelPricing(t)
	setupTopupInfoUser(t, 304, "default")
	operation_setting.Price = 7.3
	operation_setting.PayMethods = []map[string]string{{"name": "支付宝", "type": "alipay"}}
	setting.FastPayAddress = "https://fastpay.example.com/fastpay-server"
	setting.FastPayMerchantNo = "M123"
	setting.FastPayShopNo = "S123"
	setting.FastPayApiSecret = "fastpay_secret"
	operation_setting.PayAddress = "https://epay.example.com"
	operation_setting.EpayId = "10001"
	operation_setting.EpayKey = "epay_secret"

	w := httptest.NewRecorder()
	c, _ := gin.CreateTestContext(w)
	c.Set("id", 304)
	c.Request = httptest.NewRequest(http.MethodPost, "/api/user/pay", strings.NewReader(`{"amount":2,"payment_method":"fastpay_alipay"}`))
	c.Request.Header.Set("Content-Type", "application/json")
	RequestEpay(c)

	var response struct {
		Message string            `json:"message"`
		Data    map[string]string `json:"data"`
		URL     string            `json:"url"`
	}
	require.NoError(t, json.Unmarshal(w.Body.Bytes(), &response))
	require.Equal(t, "success", response.Message)
	require.Equal(t, "14.60", response.Data["amount"])
	require.Equal(t, "https://fastpay.example.com/fastpay-server/api/pay/submit", response.URL)

	var topups []model.TopUp
	require.NoError(t, model.DB.Find(&topups).Error)
	require.Len(t, topups, 1)
	require.Equal(t, model.PaymentProviderFastPay, topups[0].PaymentProvider)
	require.Equal(t, "alipay", topups[0].PaymentMethod)
	require.InDelta(t, 14.6, topups[0].Money, 0.000001)
}

func TestRequestEpayRoutesFastPayMetaMethodToFastPayValidation(t *testing.T) {
	gin.SetMode(gin.TestMode)
	confirmPaymentComplianceForTest(t)
	preservePaymentGatewaySettings(t)
	preserveChannelPricing(t)
	setupTopupInfoUser(t, 305, "default")
	operation_setting.PayMethods = []map[string]string{{"name": "支付宝", "type": "alipay"}}
	setting.FastPayAddress = "https://fastpay.example.com/fastpay-server"
	setting.FastPayMerchantNo = "M123"
	setting.FastPayShopNo = "S123"
	setting.FastPayApiSecret = "fastpay_secret"

	w := httptest.NewRecorder()
	c, _ := gin.CreateTestContext(w)
	c.Set("id", 305)
	c.Request = httptest.NewRequest(http.MethodPost, "/api/user/pay", strings.NewReader(`{"amount":2,"payment_method":"fastpay"}`))
	c.Request.Header.Set("Content-Type", "application/json")
	RequestEpay(c)

	var response struct {
		Data string `json:"data"`
	}
	require.NoError(t, json.Unmarshal(w.Body.Bytes(), &response))
	require.Equal(t, "FAST 易支付仅支持支付宝或微信支付", response.Data)
}

func TestGetTopUpInfo_EnablesOnlineTopUpWhenOnlyFastPayIsConfigured(t *testing.T) {
	gin.SetMode(gin.TestMode)
	confirmPaymentComplianceForTest(t)
	preservePaymentGatewaySettings(t)
	setupTopupInfoUser(t, 303, "default")
	setting.FastPayAddress = "https://fastpay.example.com/fastpay-server"
	setting.FastPayMerchantNo = "M123"
	setting.FastPayShopNo = "S123"
	setting.FastPayApiSecret = "fastpay_secret"
	operation_setting.PayAddress = "https://pay.example.com"
	operation_setting.EpayId = "10001"
	operation_setting.EpayKey = ""
	operation_setting.PayMethods = []map[string]string{{"name": "支付宝", "type": "alipay"}}

	w := httptest.NewRecorder()
	c, _ := gin.CreateTestContext(w)
	c.Set("id", 303)
	GetTopUpInfo(c)

	var response struct {
		Data struct {
			EnableOnlineTopUp bool `json:"enable_online_topup"`
		} `json:"data"`
	}
	require.NoError(t, json.Unmarshal(w.Body.Bytes(), &response))
	require.True(t, response.Data.EnableOnlineTopUp)
}

func TestFastPaySignAndVerify(t *testing.T) {
	secret := "MY_SECRET_KEY_123"
	params := map[string]string{
		"merchantNo": "M12345678",
		"outTradeNo": "ORDER202512050001",
		"amount":     "10.00",
		"subject":    "测试商品",
		"payType":    "wxpay",
		"timestamp":  "1733400000",
	}

	sign := GenerateFastPaySign(params, secret)
	assert.NotEmpty(t, sign)
	assert.True(t, VerifyFastPaySign(params, secret, sign))
	assert.False(t, VerifyFastPaySign(params, secret, "WRONG_SIGN"))
}

func TestFastPayNotify_InvalidSign(t *testing.T) {
	gin.SetMode(gin.TestMode)
	oldAddress := setting.FastPayAddress
	oldMerchantNo := setting.FastPayMerchantNo
	oldShopNo := setting.FastPayShopNo
	oldApiSecret := setting.FastPayApiSecret
	t.Cleanup(func() {
		setting.FastPayAddress = oldAddress
		setting.FastPayMerchantNo = oldMerchantNo
		setting.FastPayShopNo = oldShopNo
		setting.FastPayApiSecret = oldApiSecret
	})
	setting.FastPayAddress = "https://pay.example.com/fastpay-server"
	setting.FastPayMerchantNo = "M12345678"
	setting.FastPayShopNo = "S12345678"
	setting.FastPayApiSecret = "secret123"

	payload := FastPayNotifyPayload{
		MerchantNo: "M12345678",
		OrderNo:    "FP123",
		OutTradeNo: "USR1NO123",
		Amount:     "10.00",
		PayAmount:  "10.00",
		PayType:    "alipay",
		Status:     1,
		PayTime:    "2026-07-31 15:00:00",
		Timestamp:  time.Now().Unix(),
		Sign:       "invalid_sign",
	}

	bodyBytes, _ := json.Marshal(payload)
	w := httptest.NewRecorder()
	c, _ := gin.CreateTestContext(w)
	c.Request = httptest.NewRequest(http.MethodPost, "/api/user/fastpay/notify", bytes.NewBuffer(bodyBytes))

	FastPayNotify(c)

	assert.Equal(t, http.StatusOK, w.Code)
	assert.Equal(t, "fail", w.Body.String())
}

func TestBuildFastPayOrderParamsMatchesServerContract(t *testing.T) {
	cfg := &FastPayConfig{
		Address:    "https://pay.example.com/fastpay-server",
		MerchantNo: "M12345678",
		ShopNo:     "S12345678",
		ApiSecret:  "secret123",
	}

	params := buildFastPayOrderParams(
		cfg,
		"ORDER202607310001",
		"10.00",
		"测试商品",
		"alipay",
		"https://api.example.com/usage-logs",
		1785489235,
	)

	assert.Equal(t, cfg.MerchantNo, params["merchantNo"])
	assert.Equal(t, cfg.ShopNo, params["shopNo"])
	assert.NotContains(t, params, "notifyUrl")
	assert.True(t, VerifyFastPaySign(params, cfg.ApiSecret, params["sign"]))
}

func TestReadFastPayNotifyPayload_Form(t *testing.T) {
	gin.SetMode(gin.TestMode)
	secret := "secret123"
	signParams := map[string]string{
		"merchantNo": "M12345678",
		"orderNo":    "FP123",
		"outTradeNo": "USR1NO123",
		"amount":     "10.00",
		"payAmount":  "10.00",
		"payType":    "alipay",
		"status":     "1",
		"payTime":    "2026-07-31T15:00:00",
		"timestamp":  "1785489235",
	}
	form := url.Values{
		"merchantNo": {"M12345678"},
		"orderNo":    {"FP123"},
		"outTradeNo": {"USR1NO123"},
		"amount":     {"10.00"},
		"payAmount":  {"10.00"},
		"payType":    {"alipay"},
		"status":     {"1"},
		"payTime":    {"2026-07-31T15:00:00"},
		"timestamp":  {"1785489235"},
		"sign":       {GenerateFastPaySign(signParams, secret)},
	}

	c, _ := gin.CreateTestContext(httptest.NewRecorder())
	c.Request = httptest.NewRequest(http.MethodPost, "/api/user/fastpay/notify", strings.NewReader(form.Encode()))
	c.Request.Header.Set("Content-Type", "application/x-www-form-urlencoded")

	payload, rawBody, err := readFastPayNotifyPayload(c)

	assert.NoError(t, err)
	assert.Equal(t, "M12345678", payload.MerchantNo)
	assert.Equal(t, "USR1NO123", payload.OutTradeNo)
	assert.Equal(t, "1", payload.Status)
	assert.True(t, VerifyFastPaySign(fastPayNotifySignParams(payload), secret, payload.Sign))
	assert.Equal(t, form.Encode(), string(rawBody))
}

func TestGetFastPayConfigRequiresShopNo(t *testing.T) {
	oldAddress := setting.FastPayAddress
	oldMerchantNo := setting.FastPayMerchantNo
	oldShopNo := setting.FastPayShopNo
	oldApiSecret := setting.FastPayApiSecret
	t.Cleanup(func() {
		setting.FastPayAddress = oldAddress
		setting.FastPayMerchantNo = oldMerchantNo
		setting.FastPayShopNo = oldShopNo
		setting.FastPayApiSecret = oldApiSecret
	})

	setting.FastPayAddress = "https://pay.example.com/fastpay-server"
	setting.FastPayMerchantNo = "M12345678"
	setting.FastPayShopNo = ""
	setting.FastPayApiSecret = "secret123"
	assert.Nil(t, getFastPayConfig())

	setting.FastPayShopNo = "S12345678"
	assert.Equal(t, "S12345678", getFastPayConfig().ShopNo)
}

func TestIsSubscriptionFastPayTradeNo(t *testing.T) {
	assert.True(t, isSubscriptionFastPayTradeNo("SUBUSR1NO123"))
	assert.False(t, isSubscriptionFastPayTradeNo("USR1NO123"))
}

func TestGetFastPaySubmitUrl(t *testing.T) {
	assert.Equal(t, "https://api.lmm.best:9443/fastpay-server/api/pay/submit", getFastPaySubmitUrl("https://api.lmm.best:9443/fastpay-server"))
	assert.Equal(t, "https://api.lmm.best:9443/fastpay-server/api/pay/submit", getFastPaySubmitUrl("https://api.lmm.best:9443/fastpay-server/submit.php"))
	assert.Equal(t, "https://api.lmm.best:9443/fastpay-server/api/pay/submit", getFastPaySubmitUrl("https://api.lmm.best:9443/fastpay-server/api/pay/submit"))
}
