package controller

import (
	"strings"

	"github.com/LIghtJUNction/api.lmm.best/service"
	"github.com/LIghtJUNction/api.lmm.best/setting"
	"github.com/LIghtJUNction/api.lmm.best/setting/operation_setting"
)

func isPaymentComplianceConfirmed() bool {
	return operation_setting.IsPaymentComplianceConfirmed()
}

func isStripeTopUpEnabled() bool {
	if !isPaymentComplianceConfirmed() {
		return false
	}
	return strings.TrimSpace(setting.StripeApiSecret) != "" &&
		strings.TrimSpace(setting.StripeWebhookSecret) != "" &&
		strings.TrimSpace(setting.StripePriceId) != ""
}

// isStripeSubscriptionPaymentEnabled intentionally does not require the
// global wallet top-up Price ID. Subscription plans carry their own Stripe
// Price ID, so requiring the wallet product here hides otherwise valid
// subscription checkout buttons.
func isStripeSubscriptionPaymentEnabled() bool {
	if !isPaymentComplianceConfirmed() {
		return false
	}
	secret := strings.TrimSpace(setting.StripeApiSecret)
	return (strings.HasPrefix(secret, "sk_") || strings.HasPrefix(secret, "rk_")) &&
		strings.TrimSpace(setting.StripeWebhookSecret) != ""
}

func isStripeWebhookConfigured() bool {
	return strings.TrimSpace(setting.StripeWebhookSecret) != ""
}

func isStripeWebhookEnabled() bool {
	return isStripeTopUpEnabled()
}

func isCreemTopUpEnabled() bool {
	if !isPaymentComplianceConfirmed() {
		return false
	}
	products := strings.TrimSpace(setting.CreemProducts)
	return strings.TrimSpace(setting.CreemApiKey) != "" &&
		products != "" &&
		products != "[]"
}

// Subscription products are configured on each plan and therefore do not
// depend on the global wallet top-up product catalog.
func isCreemSubscriptionPaymentEnabled() bool {
	if !isPaymentComplianceConfirmed() {
		return false
	}
	return strings.TrimSpace(setting.CreemApiKey) != "" &&
		(strings.TrimSpace(setting.CreemWebhookSecret) != "" || setting.CreemTestMode)
}

func isCreemWebhookConfigured() bool {
	return strings.TrimSpace(setting.CreemWebhookSecret) != ""
}

func isCreemWebhookEnabled() bool {
	return isCreemTopUpEnabled() && isCreemWebhookConfigured()
}

func isWaffoTopUpEnabled() bool {
	if !isPaymentComplianceConfirmed() {
		return false
	}
	if !setting.WaffoEnabled {
		return false
	}

	return isWaffoWebhookConfigured()
}

func isWaffoWebhookConfigured() bool {
	if setting.WaffoSandbox {
		return strings.TrimSpace(setting.WaffoSandboxApiKey) != "" &&
			strings.TrimSpace(setting.WaffoSandboxPrivateKey) != "" &&
			strings.TrimSpace(setting.WaffoSandboxPublicCert) != ""
	}

	return strings.TrimSpace(setting.WaffoApiKey) != "" &&
		strings.TrimSpace(setting.WaffoPrivateKey) != "" &&
		strings.TrimSpace(setting.WaffoPublicCert) != ""
}

func isWaffoWebhookEnabled() bool {
	return isWaffoTopUpEnabled()
}

func isWaffoPancakeTopUpEnabled() bool {
	if !isPaymentComplianceConfirmed() {
		return false
	}
	// Presence-of-credentials = enabled. Webhook public keys ship inside
	// the SDK; mode (test/prod) is read from each event.
	merchantID, privateKey := service.WaffoPancakeCredentials()
	return merchantID != "" &&
		privateKey != "" &&
		strings.TrimSpace(setting.WaffoPancakeProductID) != ""
}

// Pancake subscriptions use the product ID stored on the plan. The global
// wallet product is unrelated and must not gate plan checkout.
func isWaffoPancakeSubscriptionPaymentEnabled() bool {
	if !isPaymentComplianceConfirmed() {
		return false
	}
	merchantID, privateKey := service.WaffoPancakeCredentials()
	return strings.TrimSpace(merchantID) != "" && strings.TrimSpace(privateKey) != ""
}

func isWaffoPancakeWebhookConfigured() bool {
	// Webhooks must remain available after new checkouts are disabled or the
	// active product is rotated. Existing orders can still complete or emit a
	// refund, and neither path needs the current product id. Keep the
	// checkout-only product requirement in isWaffoPancakeTopUpEnabled.
	merchantID, privateKey := service.WaffoPancakeCredentials()
	return merchantID != "" && privateKey != ""
}

func isWaffoPancakeWebhookEnabled() bool {
	return isPaymentComplianceConfirmed() && isWaffoPancakeWebhookConfigured()
}

func isEpayTopUpEnabled() bool {
	if !isPaymentComplianceConfirmed() {
		return false
	}
	return isEpayWebhookConfigured() && len(operation_setting.PayMethods) > 0
}

func isEpayWebhookConfigured() bool {
	return strings.TrimSpace(operation_setting.PayAddress) != "" &&
		strings.TrimSpace(operation_setting.EpayId) != "" &&
		strings.TrimSpace(operation_setting.EpayKey) != ""
}

func isEpayWebhookEnabled() bool {
	return isEpayTopUpEnabled()
}

func isFastPayTopUpEnabled() bool {
	if !isPaymentComplianceConfirmed() {
		return false
	}
	return isFastPayWebhookConfigured()
}

func isFastPayWebhookConfigured() bool {
	return strings.TrimSpace(setting.FastPayAddress) != "" &&
		strings.TrimSpace(setting.FastPayMerchantNo) != "" &&
		strings.TrimSpace(setting.FastPayShopNo) != "" &&
		strings.TrimSpace(setting.FastPayApiSecret) != ""
}

func isFastPayWebhookEnabled() bool {
	return isFastPayTopUpEnabled()
}
