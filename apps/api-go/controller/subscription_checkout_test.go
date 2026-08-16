package controller

import (
	"testing"
	"time"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/LIghtJUNction/api.lmm.best/model"
	"github.com/LIghtJUNction/api.lmm.best/setting"
	"github.com/LIghtJUNction/api.lmm.best/setting/operation_setting"
	"github.com/stretchr/testify/require"
)

func TestSubscriptionPaymentMethodsUsePlanStripeProductWithoutWalletProduct(t *testing.T) {
	paymentSetting := operation_setting.GetPaymentSetting()
	oldConfirmed := paymentSetting.ComplianceConfirmed
	oldTerms := paymentSetting.ComplianceTermsVersion
	oldAPISecret := setting.StripeApiSecret
	oldWebhookSecret := setting.StripeWebhookSecret
	oldWalletPriceID := setting.StripePriceId
	t.Cleanup(func() {
		paymentSetting.ComplianceConfirmed = oldConfirmed
		paymentSetting.ComplianceTermsVersion = oldTerms
		setting.StripeApiSecret = oldAPISecret
		setting.StripeWebhookSecret = oldWebhookSecret
		setting.StripePriceId = oldWalletPriceID
	})

	paymentSetting.ComplianceConfirmed = true
	paymentSetting.ComplianceTermsVersion = operation_setting.CurrentComplianceTermsVersion
	setting.StripeApiSecret = "sk_test_subscription"
	setting.StripeWebhookSecret = "whsec_subscription"
	// The wallet's global Stripe price is intentionally absent. Subscription
	// plans own their Stripe product and must not depend on this setting.
	setting.StripePriceId = ""

	allowBalance := false
	methods := subscriptionPaymentMethods(
		&model.User{Username: "subscription-user", Status: common.UserStatusEnabled},
		&model.SubscriptionPlan{
			PriceAmount:     10,
			AllowBalancePay: &allowBalance,
			StripePriceId:   "price_plan_specific",
		},
		time.Now(),
	)

	require.Equal(t, []string{model.PaymentMethodStripe}, methods)
}
