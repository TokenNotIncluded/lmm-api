package controller

import (
	"bytes"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/LIghtJUNction/api.lmm.best/model"
	"github.com/LIghtJUNction/api.lmm.best/setting"
	"github.com/gin-gonic/gin"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
	"github.com/waffo-com/waffo-go/config"
	"github.com/waffo-com/waffo-go/core"
	"github.com/waffo-com/waffo-go/utils"
)

func TestWaffoWebhookReceiptLogOmitsSensitiveValues(t *testing.T) {
	logLine := waffoWebhookReceiptLog("/api/waffo/webhook", "198.51.100.7", len(`{"secret":"payload"}`))

	require.Contains(t, logLine, "body_bytes=")
	require.NotContains(t, logLine, "secret")
	require.NotContains(t, logLine, "payload")
	require.NotContains(t, logLine, "signature")
}

func newWaffoRefundTestContext(t *testing.T) (*gin.Context, *httptest.ResponseRecorder, *core.WebhookHandler) {
	t.Helper()
	keys, err := utils.GenerateKeyPair()
	require.NoError(t, err)
	response := httptest.NewRecorder()
	ctx, _ := gin.CreateTestContext(response)
	ctx.Request = httptest.NewRequest(http.MethodPost, "/api/waffo/webhook", nil)
	return ctx, response, core.NewWebhookHandler(&config.WaffoConfig{PrivateKey: keys.PrivateKey})
}

func TestHandleWaffoRefundAppliesPartialFullAndReplayExactlyOnce(t *testing.T) {
	db := setupTokenControllerTestDB(t)
	require.NoError(t, db.AutoMigrate(&model.User{}, &model.TopUp{}, &model.Log{}, &model.FinanceLedgerEntry{}))
	user := model.User{Username: "waffo-refund-owner", Password: "password", Status: common.UserStatusEnabled, Quota: 100_000}
	require.NoError(t, db.Create(&user).Error)
	topUp := model.TopUp{
		UserId: user.Id, TradeNo: "WAFFO-refund-owner", Amount: 100, CreditedQuota: 100_000,
		Money: 10, SettledAmountMicros: 10_000_000, SettlementCurrency: "USD",
		PaymentMethod: model.PaymentMethodWaffo, PaymentProvider: model.PaymentProviderWaffo,
		Status: common.TopUpStatusSuccess,
	}
	require.NoError(t, db.Create(&topUp).Error)

	apply := func(refundID, amount, status string) {
		ctx, response, handler := newWaffoRefundTestContext(t)
		handleWaffoRefund(ctx, handler, &core.RefundNotificationResult{
			OrigPaymentRequestID:   topUp.TradeNo,
			AcquiringRefundOrderID: refundID,
			RefundAmount:           amount,
			RefundStatus:           status,
		})
		require.Equal(t, http.StatusOK, response.Code)
		require.JSONEq(t, `{"message":"success"}`, response.Body.String())
	}

	apply("waffo-refund-partial", "2.50", core.RefundStatusPartiallyRefunded)
	apply("waffo-refund-partial", "2.50", core.RefundStatusPartiallyRefunded)
	apply("waffo-refund-full", "7.50", core.RefundStatusFullyRefunded)

	var refreshedUser model.User
	require.NoError(t, db.First(&refreshedUser, user.Id).Error)
	assert.Zero(t, refreshedUser.Quota)
	var refreshedTopUp model.TopUp
	require.NoError(t, db.First(&refreshedTopUp, topUp.Id).Error)
	assert.Equal(t, int64(10_000_000), refreshedTopUp.RefundedAmountMicros)
	assert.Equal(t, int64(100_000), refreshedTopUp.RefundedQuota)
	var entries []model.FinanceLedgerEntry
	require.NoError(t, db.Order("id").Find(&entries).Error)
	require.Len(t, entries, 2)
	assert.Equal(t, model.PaymentProviderWaffo, entries[0].PaymentProvider)
	assert.Equal(t, model.PaymentMethodWaffo, entries[0].PaymentMethod)
}

func TestHandleWaffoRefundRejectsMismatchedOrderOrAmount(t *testing.T) {
	db := setupTokenControllerTestDB(t)
	require.NoError(t, db.AutoMigrate(&model.User{}, &model.TopUp{}, &model.Log{}, &model.FinanceLedgerEntry{}))
	user := model.User{Username: "waffo-refund-mismatch", Password: "password", Status: common.UserStatusEnabled, Quota: 100_000}
	require.NoError(t, db.Create(&user).Error)
	topUp := model.TopUp{
		UserId: user.Id, TradeNo: "WAFFO-refund-mismatch", Amount: 100, CreditedQuota: 100_000,
		Money: 10, SettledAmountMicros: 10_000_000, SettlementCurrency: "USD",
		PaymentMethod: model.PaymentMethodWaffo, PaymentProvider: model.PaymentProviderWaffo,
		Status: common.TopUpStatusSuccess,
	}
	require.NoError(t, db.Create(&topUp).Error)

	for _, result := range []*core.RefundNotificationResult{
		{OrigPaymentRequestID: "unknown-order", AcquiringRefundOrderID: "waffo-refund-unknown", RefundAmount: "1.00", RefundStatus: core.RefundStatusPartiallyRefunded},
		{OrigPaymentRequestID: topUp.TradeNo, AcquiringRefundOrderID: "waffo-refund-too-large", RefundAmount: "10.01", RefundStatus: core.RefundStatusFullyRefunded},
	} {
		ctx, response, handler := newWaffoRefundTestContext(t)
		handleWaffoRefund(ctx, handler, result)
		require.Equal(t, http.StatusOK, response.Code)
		require.JSONEq(t, `{"message":"failed"}`, response.Body.String())
	}
	// A refund must use the currency persisted at original settlement. Do not
	// guess from mutable Waffo configuration or accept the callback display
	// currency when a legacy order is incomplete.
	require.NoError(t, db.Model(&model.TopUp{}).Where("id = ?", topUp.Id).Update("settlement_currency", "").Error)
	ctx, response, handler := newWaffoRefundTestContext(t)
	handleWaffoRefund(ctx, handler, &core.RefundNotificationResult{
		OrigPaymentRequestID:   topUp.TradeNo,
		AcquiringRefundOrderID: "waffo-refund-no-currency",
		RefundAmount:           "1.00",
		RefundStatus:           core.RefundStatusPartiallyRefunded,
	})
	require.Equal(t, http.StatusOK, response.Code)
	require.JSONEq(t, `{"message":"failed"}`, response.Body.String())

	var refreshedUser model.User
	require.NoError(t, db.First(&refreshedUser, user.Id).Error)
	assert.Equal(t, 100_000, refreshedUser.Quota)
	var entries []model.FinanceLedgerEntry
	require.NoError(t, db.Find(&entries).Error)
	assert.Empty(t, entries)
}

func TestWaffoWebhookVerifiedRefundReversesLocalTopUp(t *testing.T) {
	db := setupTokenControllerTestDB(t)
	require.NoError(t, db.AutoMigrate(&model.User{}, &model.TopUp{}, &model.Log{}, &model.FinanceLedgerEntry{}))
	confirmPaymentComplianceForTest(t)
	merchantKeys, err := utils.GenerateKeyPair()
	require.NoError(t, err)
	providerKeys, err := utils.GenerateKeyPair()
	require.NoError(t, err)
	original := struct {
		enabled, sandbox               bool
		apiKey, privateKey, publicCert string
	}{
		setting.WaffoEnabled, setting.WaffoSandbox,
		setting.WaffoSandboxApiKey, setting.WaffoSandboxPrivateKey, setting.WaffoSandboxPublicCert,
	}
	t.Cleanup(func() {
		setting.WaffoEnabled = original.enabled
		setting.WaffoSandbox = original.sandbox
		setting.WaffoSandboxApiKey = original.apiKey
		setting.WaffoSandboxPrivateKey = original.privateKey
		setting.WaffoSandboxPublicCert = original.publicCert
	})
	setting.WaffoEnabled = true
	setting.WaffoSandbox = true
	setting.WaffoSandboxApiKey = "waffo-refund-test-api-key"
	setting.WaffoSandboxPrivateKey = merchantKeys.PrivateKey
	setting.WaffoSandboxPublicCert = providerKeys.PublicKey

	user := model.User{Username: "waffo-refund-webhook", Password: "password", Status: common.UserStatusEnabled, Quota: 100_000}
	require.NoError(t, db.Create(&user).Error)
	require.NoError(t, db.Create(&model.TopUp{
		UserId: user.Id, TradeNo: "WAFFO-refund-webhook", Amount: 100, CreditedQuota: 100_000,
		Money: 10, SettledAmountMicros: 10_000_000, SettlementCurrency: "USD",
		PaymentMethod: model.PaymentMethodWaffo, PaymentProvider: model.PaymentProviderWaffo,
		Status: common.TopUpStatusSuccess,
	}).Error)

	payload := []byte(`{"eventType":"REFUND_NOTIFICATION","result":{"origPaymentRequestId":"WAFFO-refund-webhook","acquiringRefundOrderId":"waffo-refund-webhook-partial","refundAmount":"2.50","refundStatus":"ORDER_PARTIALLY_REFUNDED"}}`)
	signature, err := utils.Sign(string(payload), providerKeys.PrivateKey)
	require.NoError(t, err)
	response := httptest.NewRecorder()
	ctx, _ := gin.CreateTestContext(response)
	request := httptest.NewRequest(http.MethodPost, "/api/waffo/webhook", bytes.NewReader(payload))
	request.Header.Set("X-SIGNATURE", signature)
	ctx.Request = request

	WaffoWebhook(ctx)
	require.Equal(t, http.StatusOK, response.Code)
	require.JSONEq(t, `{"message":"success"}`, response.Body.String())
	var refreshed model.User
	require.NoError(t, db.First(&refreshed, user.Id).Error)
	assert.Equal(t, 75_000, refreshed.Quota)
}
