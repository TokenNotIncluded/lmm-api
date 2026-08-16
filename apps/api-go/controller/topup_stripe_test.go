package controller

import (
	"bytes"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"testing"
	"time"

	"github.com/LIghtJUNction/api.lmm.best/model"
	"github.com/LIghtJUNction/api.lmm.best/setting"
	"github.com/gin-gonic/gin"
	"github.com/stretchr/testify/require"
	"github.com/stripe/stripe-go/v81/webhook"
)

func TestStripeWebhookReceiptLogOmitsSensitiveValues(t *testing.T) {
	logLine := stripeWebhookReceiptLog("/api/stripe/webhook", "198.51.100.7", len(`{"secret":"payload"}`))

	require.Contains(t, logLine, "body_bytes=")
	require.NotContains(t, logLine, "secret")
	require.NotContains(t, logLine, "payload")
	require.NotContains(t, logLine, "signature")
}

func TestStripeWebhookRetriesWhenLocalSettlementCannotPersist(t *testing.T) {
	setupTokenControllerTestDB(t)
	confirmPaymentComplianceForTest(t)

	originalAPISecret := setting.StripeApiSecret
	originalWebhookSecret := setting.StripeWebhookSecret
	originalPriceID := setting.StripePriceId
	t.Cleanup(func() {
		setting.StripeApiSecret = originalAPISecret
		setting.StripeWebhookSecret = originalWebhookSecret
		setting.StripePriceId = originalPriceID
	})
	setting.StripeApiSecret = "sk_test_settlement_retry"
	setting.StripeWebhookSecret = "whsec_settlement_retry"
	setting.StripePriceId = "price_settlement_retry"

	payload := []byte(`{
		"id":"evt_settlement_retry",
		"object":"event",
		"type":"checkout.session.completed",
		"data":{"object":{
			"id":"cs_settlement_retry",
			"object":"checkout.session",
			"client_reference_id":"missing-local-order",
			"status":"complete",
			"payment_status":"paid",
			"amount_total":"100",
			"amount_subtotal":"100",
			"currency":"usd"
		}}
	}`)
	signed := webhook.GenerateTestSignedPayload(&webhook.UnsignedPayload{
		Payload:   payload,
		Secret:    setting.StripeWebhookSecret,
		Timestamp: time.Now(),
	})

	response := httptest.NewRecorder()
	ctx, _ := gin.CreateTestContext(response)
	request := httptest.NewRequest(http.MethodPost, "/api/stripe/webhook", http.NoBody)
	request.Body = io.NopCloser(bytes.NewReader(payload))
	request.Header.Set("Stripe-Signature", signed.Header)
	ctx.Request = request

	StripeWebhook(ctx)
	require.Equal(t, http.StatusInternalServerError, response.Code)
}

func TestStripeWebhookRetriesOnlyPersistableFailures(t *testing.T) {
	require.True(t, stripeWebhookRetryable(errors.New("database unavailable")))
	for _, err := range []error{
		model.ErrSubscriptionOrderStatusInvalid,
		model.ErrTopUpNotFound,
		model.ErrTopUpStatusInvalid,
		model.ErrPaymentEvidenceConflict,
		model.ErrPaymentMethodMismatch,
	} {
		require.False(t, stripeWebhookRetryable(fmt.Errorf("wrapped: %w", err)))
	}
}
