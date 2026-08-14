package controller

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/Calcium-Ion/go-epay/epay"
	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/LIghtJUNction/api.lmm.best/model"
	"github.com/LIghtJUNction/api.lmm.best/setting/operation_setting"
	"github.com/gin-gonic/gin"
	"github.com/glebarez/sqlite"
	"github.com/shopspring/decimal"
	"github.com/stretchr/testify/require"
	"gorm.io/gorm"
)

func preserveChannelPricing(t *testing.T) {
	t.Helper()
	originalPrice := operation_setting.Price
	originalMethods := operation_setting.PayMethods
	originalDiscounts := operation_setting.GetPaymentSetting().AmountDiscount
	originalRatios := common.TopupGroupRatio2JSONString()
	t.Cleanup(func() {
		operation_setting.Price = originalPrice
		operation_setting.PayMethods = originalMethods
		operation_setting.GetPaymentSetting().AmountDiscount = originalDiscounts
		require.NoError(t, common.UpdateTopupGroupRatioByJSONString(originalRatios))
	})
}

func setupTopupInfoUser(t *testing.T, id int, group string) {
	t.Helper()
	previousDB := model.DB
	previousDatabaseType := common.MainDatabaseType()
	previousRedisEnabled := common.RedisEnabled
	common.SetMainDatabaseType(common.DatabaseTypeSQLite)
	common.RedisEnabled = false
	db, err := gorm.Open(sqlite.Open(":memory:"), &gorm.Config{})
	require.NoError(t, err)
	require.NoError(t, db.AutoMigrate(&model.User{}, &model.TopUp{}))
	model.DB = db
	levelOne := model.TrustLevelMinUser + 1
	require.NoError(t, db.Create(&model.User{
		Id:                 id,
		Username:           "topup-info-user",
		Password:           "test-password",
		Role:               common.RoleCommonUser,
		Status:             common.UserStatusEnabled,
		Group:              group,
		TrustLevelOverride: &levelOne,
	}).Error)
	t.Cleanup(func() {
		model.DB = previousDB
		common.SetMainDatabaseType(previousDatabaseType)
		common.RedisEnabled = previousRedisEnabled
	})
}

func TestQuoteTopUpUsesPaymentMethodUnitPriceAndSharedFormula(t *testing.T) {
	preserveChannelPricing(t)
	operation_setting.Price = 7.3
	operation_setting.PayMethods = []map[string]string{
		{"name": "支付宝", "type": "alipay"},
		{"name": "LINUX DO Credit", "type": "epay", "settlement_unit": "LDC", "unit_price": "10", "topup_ratio": "0.5"},
	}
	operation_setting.GetPaymentSetting().AmountDiscount = map[int]float64{10: 0.8}
	require.NoError(t, common.UpdateTopupGroupRatioByJSONString(`{"default":1,"vip":1.2,"ldc":0.14}`))

	legacy, err := quoteTopUp(10, "default", "alipay")
	require.NoError(t, err)
	require.True(t, legacy.Equal(decimal.RequireFromString("58.40")))

	ldc, err := quoteTopUp(1, "default", "epay")
	require.NoError(t, err)
	require.True(t, ldc.Equal(decimal.RequireFromString("5.00")))

	grouped, err := quoteTopUp(1, "ldc", "epay")
	require.NoError(t, err)
	require.True(t, grouped.Equal(decimal.RequireFromString("0.70")))

	combined, err := quoteTopUp(10, "vip", "epay")
	require.NoError(t, err)
	require.True(t, combined.Equal(decimal.RequireFromString("48.00")))
}

func TestQuoteTopUpRejectsUnknownOrInvalidConfiguredPaymentMethod(t *testing.T) {
	preserveChannelPricing(t)
	operation_setting.PayMethods = []map[string]string{
		{"name": "invalid zero", "type": "invalid-zero", "unit_price": "0"},
		{"name": "invalid text", "type": "invalid-text", "unit_price": "NaN"},
		{"name": "missing unit", "type": "missing-unit", "unit_price": "10"},
		{"name": "missing price", "type": "missing-price", "settlement_unit": "LDC"},
		{"name": "invalid unit", "type": "invalid-unit", "settlement_unit": "LDC\n", "unit_price": "10"},
		{"name": "spaced unit", "type": "spaced-unit", "settlement_unit": "L DC", "unit_price": "10"},
		{"name": "safe punctuation", "type": "safe-punctuation", "settlement_unit": ".LDC-1", "unit_price": "10"},
		{"name": "invalid topup zero", "type": "invalid-topup-zero", "topup_ratio": "0"},
		{"name": "invalid topup text", "type": "invalid-topup-text", "topup_ratio": "NaN"},
	}

	_, err := quoteTopUp(1, "default", "unknown")
	require.Error(t, err)
	_, err = quoteTopUp(1, "default", "invalid-zero")
	require.Error(t, err)
	_, err = quoteTopUp(1, "default", "invalid-text")
	require.Error(t, err)
	_, err = quoteTopUp(1, "default", "missing-unit")
	require.Error(t, err)
	_, err = quoteTopUp(1, "default", "missing-price")
	require.Error(t, err)
	_, err = quoteTopUp(1, "default", "invalid-unit")
	require.Error(t, err)
	_, err = quoteTopUp(1, "default", "spaced-unit")
	require.Error(t, err)
	punctuated, err := quoteTopUp(1, "default", "safe-punctuation")
	require.NoError(t, err)
	require.True(t, punctuated.Equal(decimal.RequireFromString("10.00")))
	_, err = quoteTopUp(1, "default", "invalid-topup-zero")
	require.Error(t, err)
	_, err = quoteTopUp(1, "default", "invalid-topup-text")
	require.Error(t, err)
}

func TestGetTopUpInfoPreservesSettlementMetadata(t *testing.T) {
	gin.SetMode(gin.TestMode)
	confirmPaymentComplianceForTest(t)
	preserveChannelPricing(t)
	setupTopupInfoUser(t, 301, "ldc")
	operation_setting.PayMethods = []map[string]string{
		{"name": "LINUX DO Credit", "type": "epay", "settlement_unit": "LDC", "unit_price": "10", "topup_ratio": "0.5"},
	}
	require.NoError(t, common.UpdateTopupGroupRatioByJSONString(`{"default":1,"ldc":0.14}`))

	w := httptest.NewRecorder()
	c, _ := gin.CreateTestContext(w)
	c.Set("id", 301)
	GetTopUpInfo(c)

	var response struct {
		Data struct {
			PayMethods      []map[string]string `json:"pay_methods"`
			TopupGroupRatio float64             `json:"topup_group_ratio"`
		} `json:"data"`
	}
	require.NoError(t, json.Unmarshal(w.Body.Bytes(), &response))
	require.Len(t, response.Data.PayMethods, 1)
	require.Equal(t, "LDC", response.Data.PayMethods[0]["settlement_unit"])
	require.Equal(t, "10", response.Data.PayMethods[0]["unit_price"])
	require.Equal(t, "0.5", response.Data.PayMethods[0]["topup_ratio"])
	require.InDelta(t, 0.14, response.Data.TopupGroupRatio, 0.000001)
}

func TestRequestAmountWithoutPaymentMethodUsesLegacyGlobalPrice(t *testing.T) {
	gin.SetMode(gin.TestMode)
	preserveChannelPricing(t)
	setupTopupInfoUser(t, 302, "default")
	operation_setting.Price = 7.3
	operation_setting.GetPaymentSetting().AmountDiscount = map[int]float64{}
	require.NoError(t, common.UpdateTopupGroupRatioByJSONString(`{"default":1}`))

	w := httptest.NewRecorder()
	c, _ := gin.CreateTestContext(w)
	c.Set("id", 302)
	c.Request = httptest.NewRequest(http.MethodPost, "/api/user/amount", strings.NewReader(`{"amount":2}`))
	c.Request.Header.Set("Content-Type", "application/json")
	RequestAmount(c)

	var response struct {
		Data string `json:"data"`
	}
	require.NoError(t, json.Unmarshal(w.Body.Bytes(), &response))
	require.Equal(t, "14.60", response.Data)
}

func TestValidateEpayCallbackRejectsSignedTypeOrMoneyMismatchAndIsIdempotent(t *testing.T) {
	pending := &model.TopUp{
		PaymentMethod:   "epay",
		PaymentProvider: model.PaymentProviderEpay,
		Money:           1.4,
		Status:          common.TopUpStatusPending,
	}

	shouldCredit, err := validateEpayCallback(pending, &epay.VerifyRes{Type: "wxpay", Money: "1.40"})
	require.Error(t, err)
	require.False(t, shouldCredit)

	shouldCredit, err = validateEpayCallback(pending, &epay.VerifyRes{Type: "epay", Money: "1.41"})
	require.Error(t, err)
	require.False(t, shouldCredit)

	shouldCredit, err = validateEpayCallback(pending, &epay.VerifyRes{Type: "epay", Money: "1.40"})
	require.NoError(t, err)
	require.True(t, shouldCredit)

	completed := *pending
	completed.Status = common.TopUpStatusSuccess
	shouldCredit, err = validateEpayCallback(&completed, &epay.VerifyRes{Type: "epay", Money: "1.40"})
	require.NoError(t, err)
	require.False(t, shouldCredit)
}
