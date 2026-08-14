package controller

import (
	"bytes"
	"crypto/md5"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"net/http"
	"net/url"
	"sort"
	"strconv"
	"strings"
	"time"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/LIghtJUNction/api.lmm.best/logger"
	"github.com/LIghtJUNction/api.lmm.best/model"
	"github.com/LIghtJUNction/api.lmm.best/setting"
	"github.com/LIghtJUNction/api.lmm.best/setting/operation_setting"
	"github.com/gin-gonic/gin"
)

type FastPayPayRequest struct {
	Amount        float64 `json:"amount"`
	PaymentMethod string  `json:"payment_method"`
	DiscountCode  string  `json:"discount_code,omitempty"`
}

type FastPayConfig struct {
	Address    string
	MerchantNo string
	ShopNo     string
	ApiSecret  string
}

func normalizeFastPayMethod(paymentMethod string) string {
	method := strings.TrimSpace(paymentMethod)
	if method == "fastpay" {
		return ""
	}
	return strings.TrimPrefix(method, "fastpay_")
}

// resolveFastPayMethod selects FAST for an explicitly prefixed method, or as
// the default online gateway when FAST is configured and Epay is not. Plain
// payment methods remain on Epay when both gateways are configured.
func resolveFastPayMethod(paymentMethod string) (string, bool) {
	method := strings.TrimSpace(paymentMethod)
	normalized := normalizeFastPayMethod(method)
	if method == "fastpay" || strings.HasPrefix(method, "fastpay_") {
		return normalized, true
	}

	if getFastPayConfig() == nil {
		return normalized, false
	}
	if !isEpayWebhookConfigured() || strings.Contains(strings.ToLower(operation_setting.PayAddress), "fastpay") {
		return normalized, true
	}

	return normalized, false
}

func isSupportedFastPayMethod(paymentMethod string) bool {
	return paymentMethod == "alipay" || paymentMethod == "wxpay"
}

func getFastPayConfig() *FastPayConfig {
	addr := strings.TrimSpace(setting.FastPayAddress)
	merchantNo := strings.TrimSpace(setting.FastPayMerchantNo)
	shopNo := strings.TrimSpace(setting.FastPayShopNo)
	secret := strings.TrimSpace(setting.FastPayApiSecret)

	// Fallback to operation_setting PayAddress/EpayId/EpayKey if PayAddress contains fastpay
	if addr == "" && strings.Contains(strings.ToLower(operation_setting.PayAddress), "fastpay") {
		addr = strings.TrimSpace(operation_setting.PayAddress)
	}
	if merchantNo == "" && strings.Contains(strings.ToLower(operation_setting.PayAddress), "fastpay") {
		merchantNo = strings.TrimSpace(operation_setting.EpayId)
	}
	if secret == "" && strings.Contains(strings.ToLower(operation_setting.PayAddress), "fastpay") {
		secret = strings.TrimSpace(operation_setting.EpayKey)
	}

	if addr == "" || merchantNo == "" || shopNo == "" || secret == "" {
		return nil
	}

	return &FastPayConfig{
		Address:    addr,
		MerchantNo: merchantNo,
		ShopNo:     shopNo,
		ApiSecret:  secret,
	}
}

func getFastPaySubmitUrl(rawAddr string) string {
	u := strings.TrimSuffix(strings.TrimSpace(rawAddr), "/submit.php")
	u = strings.TrimRight(u, "/")
	if !strings.HasSuffix(u, "/api/pay/submit") {
		u += "/api/pay/submit"
	}
	return u
}

// GenerateFastPaySign generates MD5 signature according to FastPay specification:
// 1. Sort all non-null params (excluding "sign") by ASCII order.
// 2. Format as key1=val1&key2=val2...
// 3. Append &key=secret.
// 4. MD5 hash and upper-case.
func GenerateFastPaySign(params map[string]string, secret string) string {
	keys := make([]string, 0, len(params))
	for k, v := range params {
		if k == "sign" || v == "" {
			continue
		}
		keys = append(keys, k)
	}
	sort.Strings(keys)

	var sb strings.Builder
	for i, k := range keys {
		if i > 0 {
			sb.WriteString("&")
		}
		sb.WriteString(k)
		sb.WriteString("=")
		sb.WriteString(params[k])
	}
	sb.WriteString("&key=")
	sb.WriteString(secret)

	hash := md5.Sum([]byte(sb.String()))
	return strings.ToUpper(hex.EncodeToString(hash[:]))
}

func VerifyFastPaySign(params map[string]string, secret string, expectedSign string) bool {
	if secret == "" || expectedSign == "" {
		return false
	}
	sign := GenerateFastPaySign(params, secret)
	return strings.EqualFold(sign, expectedSign)
}

func buildFastPayOrderParams(cfg *FastPayConfig, outTradeNo, amount, subject, payType, returnUrl string, timestamp int64) map[string]string {
	params := map[string]string{
		"merchantNo": cfg.MerchantNo,
		"shopNo":     cfg.ShopNo,
		"outTradeNo": outTradeNo,
		"amount":     amount,
		"subject":    subject,
		"payType":    payType,
		"returnUrl":  returnUrl,
		"timestamp":  strconv.FormatInt(timestamp, 10),
	}
	params["sign"] = GenerateFastPaySign(params, cfg.ApiSecret)
	return params
}

func parsePayRequest(c *gin.Context, amount *float64, paymentMethod, discountCode *string) error {
	// Check if values were pre-parsed by a parent handler (e.g. RequestEpay delegating to RequestFastPay)
	if v, exists := c.Get("parsed_amount"); exists {
		if amt, ok := v.(float64); ok && amt > 0 {
			*amount = amt
		}
	}
	if v, exists := c.Get("parsed_payment_method"); exists {
		if pm, ok := v.(string); ok && pm != "" {
			*paymentMethod = pm
		}
	}
	if v, exists := c.Get("parsed_discount_code"); exists {
		if code, ok := v.(string); ok {
			*discountCode = code
		}
	}

	// If we already have values from context, no need to read body
	if *amount > 0 && *paymentMethod != "" {
		return nil
	}

	// Otherwise try to parse from request body/form/query
	var req struct {
		Amount        float64 `json:"amount" form:"amount"`
		PaymentMethod string  `json:"payment_method" form:"payment_method"`
		DiscountCode  string  `json:"discount_code" form:"discount_code"`
	}

	_ = c.ShouldBind(&req)

	if *amount <= 0 && req.Amount > 0 {
		*amount = req.Amount
	}
	if *paymentMethod == "" && req.PaymentMethod != "" {
		*paymentMethod = req.PaymentMethod
	}
	if *discountCode == "" && req.DiscountCode != "" {
		*discountCode = req.DiscountCode
	}

	if *amount <= 0 {
		if amtStr := c.PostForm("amount"); amtStr != "" {
			*amount, _ = strconv.ParseFloat(amtStr, 64)
		} else if amtStr := c.Query("amount"); amtStr != "" {
			*amount, _ = strconv.ParseFloat(amtStr, 64)
		}
	}

	if *paymentMethod == "" {
		if pm := c.PostForm("payment_method"); pm != "" {
			*paymentMethod = pm
		} else if pm := c.Query("payment_method"); pm != "" {
			*paymentMethod = pm
		}
	}

	if *amount <= 0 {
		return fmt.Errorf("amount is required and must be > 0")
	}
	return nil
}

func RequestFastPay(c *gin.Context) {
	if !requirePaymentCompliance(c) {
		return
	}

	var req FastPayPayRequest
	if err := parsePayRequest(c, &req.Amount, &req.PaymentMethod, &req.DiscountCode); err != nil {
		logger.LogWarn(c.Request.Context(), fmt.Sprintf("FAST易支付 参数解包失败 error=%q", err.Error()))
		c.JSON(http.StatusOK, gin.H{"message": "error", "data": fmt.Sprintf("参数错误: %s", err.Error())})
		return
	}
	req.PaymentMethod = normalizeFastPayMethod(req.PaymentMethod)
	if !isSupportedFastPayMethod(req.PaymentMethod) {
		c.JSON(http.StatusOK, gin.H{"message": "error", "data": "FAST 易支付仅支持支付宝或微信支付"})
		return
	}
	if !requirePaymentMethodAvailable(c, req.PaymentMethod) {
		return
	}
	int64Amount := int64(req.Amount)
	if int64Amount < getMinTopup() {
		c.JSON(http.StatusOK, gin.H{"message": "error", "data": fmt.Sprintf("充值数量不能小于 %d", getMinTopup())})
		return
	}
	if !requirePaymentMethodTopUpWithinLimit(c, req.PaymentMethod, int64Amount) {
		return
	}

	cfg := getFastPayConfig()
	if cfg == nil {
		c.JSON(http.StatusOK, gin.H{"message": "error", "data": "当前管理员未配置 FAST 易支付信息"})
		return
	}

	id := c.GetInt("id")
	group, err := getTopupUserGroup(id)
	if err != nil {
		c.JSON(http.StatusOK, gin.H{"message": "error", "data": "获取用户分组失败"})
		return
	}
	payMoneyDecimal, discountCode, err := quoteLegacyTopUpWithDiscount(int64Amount, group, req.DiscountCode)
	if err != nil {
		c.JSON(http.StatusOK, gin.H{"message": "error", "data": "优惠码无效"})
		return
	}
	payMoney := payMoneyDecimal.InexactFloat64()
	if payMoney < 0.01 {
		c.JSON(http.StatusOK, gin.H{"message": "error", "data": "充值金额过低"})
		return
	}

	returnUrl := paymentReturnPath("/usage-logs")
	tradeNo := fmt.Sprintf("USR%dNO%s%d", id, common.GetRandomString(6), time.Now().Unix())
	paymentAmount := strconv.FormatFloat(payMoney, 'f', 2, 64)

	params := buildFastPayOrderParams(
		cfg,
		tradeNo,
		paymentAmount,
		fmt.Sprintf("TUC%d", int64Amount),
		req.PaymentMethod,
		returnUrl,
		time.Now().Unix(),
	)

	amount, creditedQuota := topUpOrderAmounts(int64Amount)
	expectedAmountMicros, err := monetaryStringToMicros(paymentAmount)
	if err != nil {
		logger.LogError(c.Request.Context(), fmt.Sprintf("FAST易支付 结算金额无效 user_id=%d trade_no=%s error=%q", id, tradeNo, err.Error()))
		c.JSON(http.StatusOK, gin.H{"message": "error", "data": "支付金额无效"})
		return
	}

	topUp := &model.TopUp{
		UserId:               id,
		Amount:               amount,
		CreditedQuota:        creditedQuota,
		ExpectedAmountMicros: expectedAmountMicros,
		Money:                monetaryMicrosToFloat(expectedAmountMicros),
		TradeNo:              tradeNo,
		PaymentMethod:        req.PaymentMethod,
		PaymentProvider:      model.PaymentProviderFastPay,
		DiscountCodeId:       discountCodeID(discountCode),
		DiscountPercent:      discountPercent(discountCode),
		CreateTime:           time.Now().Unix(),
		Status:               common.TopUpStatusPending,
	}
	if err := topUp.Insert(); err != nil {
		logger.LogError(c.Request.Context(), fmt.Sprintf("FAST易支付 创建充值订单失败 user_id=%d trade_no=%s error=%q", id, tradeNo, err.Error()))
		c.JSON(http.StatusOK, gin.H{"message": "error", "data": "创建订单失败"})
		return
	}

	submitUrl := getFastPaySubmitUrl(cfg.Address)

	c.JSON(http.StatusOK, gin.H{
		"message": "success",
		"data":    params,
		"url":     submitUrl,
	})
}

type FastPayNotifyPayload struct {
	MerchantNo string      `json:"merchantNo"`
	OrderNo    string      `json:"orderNo"`
	OutTradeNo string      `json:"outTradeNo"`
	Amount     interface{} `json:"amount"`
	PayAmount  interface{} `json:"payAmount"`
	PayType    string      `json:"payType"`
	Status     interface{} `json:"status"`
	PayTime    string      `json:"payTime"`
	Timestamp  interface{} `json:"timestamp"`
	Sign       string      `json:"sign"`
}

func readFastPayNotifyPayload(c *gin.Context) (FastPayNotifyPayload, []byte, error) {
	bodyBytes, err := common.ReadAllLimit(c.Request.Body, common.GetAnonymousRequestBodyLimitBytes())
	if err != nil || len(bytes.TrimSpace(bodyBytes)) == 0 {
		return FastPayNotifyPayload{}, nil, fmt.Errorf("empty callback body")
	}

	var payload FastPayNotifyPayload
	contentType := strings.ToLower(c.GetHeader("Content-Type"))
	if strings.Contains(contentType, "application/json") || bytes.HasPrefix(bytes.TrimSpace(bodyBytes), []byte("{")) {
		if err := json.Unmarshal(bodyBytes, &payload); err != nil {
			return FastPayNotifyPayload{}, nil, err
		}
		return payload, bodyBytes, nil
	}

	values, err := url.ParseQuery(string(bodyBytes))
	if err != nil {
		return FastPayNotifyPayload{}, nil, err
	}
	payload = FastPayNotifyPayload{
		MerchantNo: values.Get("merchantNo"),
		OrderNo:    values.Get("orderNo"),
		OutTradeNo: values.Get("outTradeNo"),
		Amount:     values.Get("amount"),
		PayAmount:  values.Get("payAmount"),
		PayType:    values.Get("payType"),
		Status:     values.Get("status"),
		PayTime:    values.Get("payTime"),
		Timestamp:  values.Get("timestamp"),
		Sign:       values.Get("sign"),
	}
	return payload, bodyBytes, nil
}

func fastPayNotifySignParams(payload FastPayNotifyPayload) map[string]string {
	return map[string]string{
		"merchantNo": payload.MerchantNo,
		"orderNo":    payload.OrderNo,
		"outTradeNo": payload.OutTradeNo,
		"amount":     fmt.Sprintf("%v", payload.Amount),
		"payAmount":  fmt.Sprintf("%v", payload.PayAmount),
		"payType":    payload.PayType,
		"status":     fmt.Sprintf("%v", payload.Status),
		"payTime":    payload.PayTime,
		"timestamp":  fmt.Sprintf("%v", payload.Timestamp),
	}
}

func fastPaySettlementMicros(payload FastPayNotifyPayload) (int64, error) {
	value := strings.TrimSpace(fmt.Sprintf("%v", payload.PayAmount))
	if value == "" || value == "<nil>" {
		return 0, fmt.Errorf("signed payAmount is required")
	}
	return monetaryStringToMicros(value)
}

func FastPayNotify(c *gin.Context) {
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
		logger.LogWarn(c.Request.Context(), fmt.Sprintf("FAST易支付 回调验签失败 outTradeNo=%s client_ip=%s", payload.OutTradeNo, c.ClientIP()))
		_, _ = c.Writer.Write([]byte("fail"))
		return
	}
	if cfg == nil || strings.TrimSpace(payload.MerchantNo) != cfg.MerchantNo {
		logger.LogWarn(c.Request.Context(), fmt.Sprintf("FAST易支付 回调商户号不匹配 outTradeNo=%s merchantNo=%s client_ip=%s", payload.OutTradeNo, payload.MerchantNo, c.ClientIP()))
		_, _ = c.Writer.Write([]byte("fail"))
		return
	}

	statusStr := fmt.Sprintf("%v", payload.Status)
	if statusStr != "1" && statusStr != "SUCCESS" {
		_, _ = c.Writer.Write([]byte("fail"))
		return
	}
	if isSubscriptionFastPayTradeNo(payload.OutTradeNo) {
		completeSubscriptionFastPayNotify(c, payload, string(bodyBytes))
		return
	}

	LockOrder(payload.OutTradeNo)
	defer UnlockOrder(payload.OutTradeNo)

	topUp := model.GetTopUpByTradeNo(payload.OutTradeNo)
	if topUp == nil {
		logger.LogWarn(c.Request.Context(), fmt.Sprintf("FAST易支付 回调订单不存在 trade_no=%s client_ip=%s", payload.OutTradeNo, c.ClientIP()))
		_, _ = c.Writer.Write([]byte("fail"))
		return
	}

	if topUp.PaymentProvider != model.PaymentProviderFastPay {
		logger.LogWarn(c.Request.Context(), fmt.Sprintf("FAST易支付 订单支付网关不匹配 trade_no=%s order_provider=%s client_ip=%s", payload.OutTradeNo, topUp.PaymentProvider, c.ClientIP()))
		_, _ = c.Writer.Write([]byte("fail"))
		return
	}
	if payload.PayType != "" && topUp.PaymentMethod != "" && payload.PayType != topUp.PaymentMethod {
		logger.LogWarn(c.Request.Context(), fmt.Sprintf("FAST易支付 回调支付方式不匹配 trade_no=%s expected=%s actual=%s client_ip=%s", topUp.TradeNo, topUp.PaymentMethod, payload.PayType, c.ClientIP()))
		_, _ = c.Writer.Write([]byte("fail"))
		return
	}
	settledAmountMicros, err := fastPaySettlementMicros(payload)
	if err != nil {
		logger.LogWarn(c.Request.Context(), fmt.Sprintf("FAST易支付 回调金额无效 trade_no=%s client_ip=%s error=%q", topUp.TradeNo, c.ClientIP(), err.Error()))
		_, _ = c.Writer.Write([]byte("fail"))
		return
	}
	wasPending := topUp.Status == common.TopUpStatusPending
	completed, err := model.CompleteExternalTopUp(model.ExternalTopUpSettlement{
		TradeNo:               payload.OutTradeNo,
		PaymentProvider:       model.PaymentProviderFastPay,
		PaymentMethod:         payload.PayType,
		SettledAmountMicros:   settledAmountMicros,
		ProviderTransactionId: payload.OrderNo,
	})
	if err != nil {
		logger.LogError(c.Request.Context(), fmt.Sprintf("FAST易支付 充值结算失败 trade_no=%s client_ip=%s error=%q", topUp.TradeNo, c.ClientIP(), err.Error()))
		_, _ = c.Writer.Write([]byte("fail"))
		return
	}
	if wasPending {
		logger.LogInfo(c.Request.Context(), fmt.Sprintf("FAST易支付 充值成功 trade_no=%s user_id=%d quota_to_add=%d money=%.2f", completed.TradeNo, completed.UserId, completed.CreditedQuota, completed.Money))
		model.RecordTopupLog(completed.UserId, fmt.Sprintf("使用在线充值成功，充值金额: %v，支付金额：%f", logger.LogQuota(int(completed.CreditedQuota)), completed.Money), c.ClientIP(), completed.PaymentMethod, "fastpay")
	}

	_, _ = c.Writer.Write([]byte("success"))
}
