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
