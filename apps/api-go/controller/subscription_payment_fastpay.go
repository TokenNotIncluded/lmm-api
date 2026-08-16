package controller

import (
	"fmt"
	"net/http"
	"strconv"
	"strings"
	"time"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/LIghtJUNction/api.lmm.best/logger"
	"github.com/LIghtJUNction/api.lmm.best/model"
	"github.com/gin-gonic/gin"
)

type SubscriptionFastPayPayRequest struct {
	PlanId        int    `json:"plan_id"`
	PaymentMethod string `json:"payment_method"`
}

func SubscriptionRequestFastPay(c *gin.Context) {
	if !requirePaymentCompliance(c) {
		return
	}

	var req SubscriptionFastPayPayRequest
	// Check if values were pre-parsed by SubscriptionRequestEpay
	if v, exists := c.Get("parsed_plan_id"); exists {
		if planId, ok := v.(int); ok {
			req.PlanId = planId
		}
	}
	if v, exists := c.Get("parsed_payment_method"); exists {
		if pm, ok := v.(string); ok {
			req.PaymentMethod = pm
		}
	}
	// If not pre-parsed, try reading body directly
	if req.PlanId <= 0 {
		_ = c.ShouldBindJSON(&req)
	}
	if req.PlanId <= 0 {
		common.ApiErrorMsg(c, "参数错误")
		return
	}
	req.PaymentMethod = normalizeFastPayMethod(req.PaymentMethod)
	if !isSupportedFastPayMethod(req.PaymentMethod) {
		common.ApiErrorMsg(c, "FAST 易支付仅支持支付宝或微信支付")
		return
	}
	plan, err := model.GetSubscriptionPlanById(req.PlanId)
	if err != nil {
		common.ApiError(c, err)
		return
	}
	if !plan.Enabled {
		common.ApiErrorMsg(c, "套餐未启用")
		return
	}
	if plan.PriceAmount < 0.01 {
		common.ApiErrorMsg(c, "套餐金额过低")
		return
	}
	if !requireSubscriptionPaymentMethodAvailable(c, plan, req.PaymentMethod) {
		return
	}

	cfg := getFastPayConfig()
	if cfg == nil {
		common.ApiErrorMsg(c, "当前管理员未配置 FAST 易支付信息")
		return
	}

	userId := c.GetInt("id")
	if plan.MaxPurchasePerUser > 0 {
		count, err := model.CountUserSubscriptionsByPlan(userId, plan.Id)
		if err != nil {
			common.ApiError(c, err)
			return
		}
		if count >= int64(plan.MaxPurchasePerUser) {
			common.ApiErrorMsg(c, "已达到该套餐购买上限")
			return
		}
	}

	returnUrl := paymentReturnPath("/wallet?pay=success")
	tradeNo := fmt.Sprintf("SUBUSR%dNO%s%d", userId, common.GetRandomString(6), time.Now().Unix())

	order := &model.SubscriptionOrder{
		UserId:          userId,
		PlanId:          plan.Id,
		Money:           plan.PriceAmount,
		TradeNo:         tradeNo,
		PaymentMethod:   req.PaymentMethod,
		PaymentProvider: model.PaymentProviderFastPay,
		CreateTime:      time.Now().Unix(),
		Status:          common.TopUpStatusPending,
	}
	if err := order.Insert(); err != nil {
		common.ApiErrorMsg(c, "创建订单失败")
		return
	}

	params := buildFastPayOrderParams(
		cfg,
		tradeNo,
		strconv.FormatFloat(plan.PriceAmount, 'f', 2, 64),
		fmt.Sprintf("SUB:%s", plan.Title),
		req.PaymentMethod,
		returnUrl,
		time.Now().Unix(),
	)

	submitUrl := getFastPaySubmitUrl(cfg.Address)

	c.JSON(http.StatusOK, gin.H{"message": "success", "data": params, "url": submitUrl})
}

func SubscriptionFastPayNotify(c *gin.Context) {
	payload, bodyBytes, err := readFastPayNotifyPayload(c)
	if err != nil {
		_, _ = c.Writer.Write([]byte("fail"))
		return
	}

	params := fastPayNotifySignParams(payload)

	cfg := getFastPayConfig()
	secret := ""
	if cfg != nil {
		secret = cfg.ApiSecret
	}

	if !VerifyFastPaySign(params, secret, payload.Sign) {
		logger.LogWarn(c.Request.Context(), fmt.Sprintf("FAST易支付 订阅回调验签失败 outTradeNo=%s client_ip=%s", payload.OutTradeNo, c.ClientIP()))
		_, _ = c.Writer.Write([]byte("fail"))
		return
	}

	statusStr := fmt.Sprintf("%v", payload.Status)
	if statusStr != "1" && statusStr != "SUCCESS" {
		_, _ = c.Writer.Write([]byte("fail"))
		return
	}
	completeSubscriptionFastPayNotify(c, payload, string(bodyBytes))
}

func isSubscriptionFastPayTradeNo(tradeNo string) bool {
	return strings.HasPrefix(tradeNo, "SUBUSR")
}

func completeSubscriptionFastPayNotify(c *gin.Context, payload FastPayNotifyPayload, rawPayload string) {
	LockOrder(payload.OutTradeNo)
	defer UnlockOrder(payload.OutTradeNo)

	if err := model.CompleteSubscriptionOrder(payload.OutTradeNo, rawPayload, model.PaymentProviderFastPay, payload.PayType); err != nil {
		_, _ = c.Writer.Write([]byte("fail"))
		return
	}

	_, _ = c.Writer.Write([]byte("success"))
}
