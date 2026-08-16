package controller

import (
	"strings"
	"time"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/LIghtJUNction/api.lmm.best/model"
	"github.com/LIghtJUNction/api.lmm.best/service"
	"github.com/LIghtJUNction/api.lmm.best/setting"
	"github.com/LIghtJUNction/api.lmm.best/setting/operation_setting"
	"github.com/gin-gonic/gin"
)

// subscriptionPaymentMethods reports checkout methods for one plan. Wallet
// top-up availability cannot be reused here: Stripe, Creem and Pancake
// subscriptions use a product ID stored on the plan, while wallet top-ups
// have separate global product settings.
func subscriptionPaymentMethods(user *model.User, plan *model.SubscriptionPlan, now time.Time) []string {
	if user == nil || plan == nil || !operation_setting.IsPaymentComplianceConfirmed() {
		return []string{}
	}

	methods := make([]string, 0, 4)
	// Balance checkout is subject to the same payment restriction gate as its
	// endpoint. Do not advertise it to an account that the balance route will
	// reject, while still allowing explicitly-audienced external methods below.
	if !model.IsPaymentRestricted(user) && (plan.AllowBalancePay == nil || *plan.AllowBalancePay) {
		methods = append(methods, model.PaymentMethodBalance)
	}

	appendIfAvailable := func(method string, configured bool) {
		if configured && isPaymentMethodAvailableForUser(user, method, now) {
			methods = append(methods, method)
		}
	}

	appendIfAvailable(model.PaymentMethodStripe,
		strings.TrimSpace(plan.StripePriceId) != "" &&
			(strings.HasPrefix(strings.TrimSpace(setting.StripeApiSecret), "sk_") ||
				strings.HasPrefix(strings.TrimSpace(setting.StripeApiSecret), "rk_")) &&
			strings.TrimSpace(setting.StripeWebhookSecret) != "")
	appendIfAvailable(model.PaymentMethodCreem,
		strings.TrimSpace(plan.CreemProductId) != "" &&
			strings.TrimSpace(setting.CreemApiKey) != "" &&
			(strings.TrimSpace(setting.CreemWebhookSecret) != "" || setting.CreemTestMode))

	merchantID, privateKey := service.WaffoPancakeCredentials()
	appendIfAvailable(model.PaymentMethodWaffoPancake,
		strings.TrimSpace(plan.WaffoPancakeProductId) != "" &&
			strings.TrimSpace(merchantID) != "" && strings.TrimSpace(privateKey) != "")

	// Generic ePay/FAST methods are selected by their configured type. They do
	// not need a product ID on the plan because the plan amount is sent as the
	// checkout amount.
	genericGatewayAvailable := isEpayTopUpEnabled() || isFastPayTopUpEnabled()
	if genericGatewayAvailable {
		seen := make(map[string]struct{}, len(methods))
		for _, method := range methods {
			seen[method] = struct{}{}
		}
		for _, configured := range operation_setting.PayMethods {
			method := strings.TrimSpace(configured["type"])
			if method == "" || method == model.PaymentMethodStripe ||
				method == model.PaymentMethodCreem || method == model.PaymentMethodWaffo ||
				method == model.PaymentMethodWaffoPancake || method == model.PaymentMethodBalance {
				continue
			}
			if _, ok := seen[method]; ok ||
				(!isEpayTopUpEnabled() && !isSupportedFastPayMethod(method)) ||
				!isPaymentMethodAvailableForUser(user, method, now) {
				continue
			}
			methods = append(methods, method)
			seen[method] = struct{}{}
		}
	}

	return methods
}

// requireSubscriptionPaymentMethodAvailable keeps the checkout endpoint and
// the public plan catalog on the same server-side source of truth. A provider
// may be globally configured for wallet top-ups but still be unavailable for
// this plan (missing its product ID) or for this user (audience/unlock rules).
func requireSubscriptionPaymentMethodAvailable(c *gin.Context, plan *model.SubscriptionPlan, method string) bool {
	if plan == nil {
		common.ApiErrorMsg(c, "套餐不存在")
		return false
	}

	var user *model.User
	if cached, ok := c.Get("payment_user"); ok {
		user, _ = cached.(*model.User)
	}
	if user == nil {
		var err error
		user, err = model.GetUserById(c.GetInt("id"), false)
		if err != nil || user == nil {
			common.ApiErrorMsg(c, "获取用户信息失败")
			return false
		}
	}

	for _, available := range subscriptionPaymentMethods(user, plan, time.Now()) {
		if available == method {
			return true
		}
	}
	common.ApiErrorMsg(c, "该支付方式不可用于此套餐")
	return false
}
