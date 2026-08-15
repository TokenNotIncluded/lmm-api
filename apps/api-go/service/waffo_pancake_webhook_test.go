package service

import (
	"testing"

	"github.com/LIghtJUNction/api.lmm.best/setting"
	"github.com/stretchr/testify/require"
)

func TestWaffoPancakeWebhookActionForEvent(t *testing.T) {
	tests := map[string]WaffoPancakeWebhookAction{
		"order.completed":                WaffoPancakeWebhookActionOrderCompleted,
		"subscription.activated":         WaffoPancakeWebhookActionSubscriptionActivated,
		"subscription.payment_succeeded": WaffoPancakeWebhookActionSubscriptionPaymentSucceeded,
		"refund.succeeded":               WaffoPancakeWebhookActionRefundSucceeded,
		"refund.failed":                  WaffoPancakeWebhookActionRefundFailed,
		"subscription.canceled":          WaffoPancakeWebhookActionIgnore,
		"":                               WaffoPancakeWebhookActionIgnore,
	}
	for eventType, expected := range tests {
		t.Run(eventType, func(t *testing.T) {
			require.Equal(t, expected, WaffoPancakeWebhookActionForEvent(eventType))
		})
	}
}

func TestValidateWaffoPancakeWebhookEventRejectsContradictoryStatuses(t *testing.T) {
	tests := []struct {
		name      string
		eventType string
		data      WaffoPancakeWebhookData
		wantError string
	}{
		{
			name:      "completed order with failed payment",
			eventType: "order.completed",
			data: WaffoPancakeWebhookData{
				OrderStatus:   "completed",
				PaymentStatus: "failed",
			},
			wantError: "paymentStatus mismatch",
		},
		{
			name:      "completed event with pending order",
			eventType: "order.completed",
			data: WaffoPancakeWebhookData{
				OrderStatus:   "pending",
				PaymentStatus: "succeeded",
			},
			wantError: "orderStatus mismatch",
		},
		{
			name:      "successful refund marked failed",
			eventType: "refund.succeeded",
			data:      WaffoPancakeWebhookData{RefundStatus: "failed"},
			wantError: "refundStatus mismatch",
		},
		{
			name:      "failed refund marked successful",
			eventType: "refund.failed",
			data:      WaffoPancakeWebhookData{RefundStatus: "succeeded"},
			wantError: "refundStatus mismatch",
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			err := ValidateWaffoPancakeWebhookEvent(&WaffoPancakeWebhookEvent{
				EventType: tt.eventType,
				Data:      tt.data,
			})
			require.ErrorContains(t, err, tt.wantError)
		})
	}
}

func TestValidateWaffoPancakeWebhookEventAllowsOmittedOptionalStatuses(t *testing.T) {
	for _, eventType := range []string{
		"order.completed",
		"subscription.activated",
		"subscription.payment_succeeded",
		"refund.succeeded",
		"refund.failed",
	} {
		t.Run(eventType, func(t *testing.T) {
			require.NoError(t, ValidateWaffoPancakeWebhookEvent(&WaffoPancakeWebhookEvent{
				EventType: eventType,
			}))
		})
	}
}

func TestWaffoPancakeCredentialsPreferSettingsAndFallBackToOfficialEnv(t *testing.T) {
	originalMerchantID := setting.WaffoPancakeMerchantID
	originalPrivateKey := setting.WaffoPancakePrivateKey
	t.Cleanup(func() {
		setting.WaffoPancakeMerchantID = originalMerchantID
		setting.WaffoPancakePrivateKey = originalPrivateKey
	})
	t.Setenv("WAFFO_MERCHANT_ID", "env-merchant")
	t.Setenv("WAFFO_PRIVATE_KEY", "env-private-key")

	setting.WaffoPancakeMerchantID = ""
	setting.WaffoPancakePrivateKey = ""
	merchantID, privateKey := WaffoPancakeCredentials()
	require.Equal(t, "env-merchant", merchantID)
	require.Equal(t, "env-private-key", privateKey)

	setting.WaffoPancakeMerchantID = "stored-merchant"
	setting.WaffoPancakePrivateKey = "stored-private-key"
	merchantID, privateKey = WaffoPancakeCredentials()
	require.Equal(t, "stored-merchant", merchantID)
	require.Equal(t, "stored-private-key", privateKey)
}
