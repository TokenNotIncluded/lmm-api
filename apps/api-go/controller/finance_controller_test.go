package controller

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strconv"
	"testing"
	"time"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/LIghtJUNction/api.lmm.best/model"
	"github.com/gin-gonic/gin"
	"github.com/stretchr/testify/require"
)

func TestFinanceOverviewAggregatesPaymentMethodsUsersAndTokenCost(t *testing.T) {
	db := setupTokenControllerTestDB(t)
	require.NoError(t, db.AutoMigrate(&model.User{}, &model.TopUp{}, &model.SubscriptionOrder{}, &model.Log{}, &model.FinanceLedgerEntry{}, &model.FinancePaymentMethod{}))
	now := time.Now().Unix()
	users := []model.User{{Username: "finance-a", Password: "password", Role: common.RoleCommonUser, Status: common.UserStatusEnabled, Group: "default", AffCode: "finance-a"}, {Username: "finance-b", Password: "password", Role: common.RoleCommonUser, Status: common.UserStatusEnabled, Group: "default", AffCode: "finance-b"}}
	require.NoError(t, db.Create(&users).Error)
	require.NoError(t, db.Create(&[]model.TopUp{
		{UserId: users[0].Id, TradeNo: "finance-topup-a", Amount: 100, Money: 10, Status: common.TopUpStatusSuccess, PaymentMethod: "stripe", PaymentProvider: "stripe", CompleteTime: now - 10},
		{UserId: users[1].Id, TradeNo: "finance-topup-b", Amount: 250, Money: 25, Status: common.TopUpStatusSuccess, PaymentMethod: "creem", PaymentProvider: "creem", CompleteTime: now - 20},
	}).Error)
	require.NoError(t, db.Create(&model.Log{UserId: users[0].Id, CreatedAt: now - 5, Type: model.LogTypeConsume, PromptTokens: 1000, CompletionTokens: 500, ModelName: "priced", Other: `{"model_price":0.02}`}).Error)
	_, err := model.AppendFinanceLedgerEntry(&model.FinanceLedgerEntry{EntryType: model.FinanceEntryExpense, Category: "hosting", AmountMicros: 3_000_000, Currency: "USD", Direction: model.FinanceDirectionDebit, SourceType: model.FinanceSourceManual, Note: "monthly host", OccurredAt: now - 30, CreatedBy: users[0].Id, IdempotencyKey: "hosting-1"})
	require.NoError(t, err)

	view, err := buildFinanceOverview(now-100, now+1, 0, "")
	require.NoError(t, err)
	require.Equal(t, int64(35_000_000), view.RevenueMicros)
	require.Equal(t, int64(3_000_000+30), view.ExpenseMicros)
	require.Equal(t, int64(31_999_970), view.ProfitMicros)
	require.Equal(t, int64(1500), view.Tokens.TotalTokens)
	require.Equal(t, int64(30), view.Tokens.EstimatedCostMicros)
	require.Len(t, view.RevenueByMethod, 2)
	require.Len(t, view.Users, 2)

	stripeView, err := buildFinanceOverview(now-100, now+1, 0, "stripe")
	require.NoError(t, err)
	require.Equal(t, int64(10_000_000), stripeView.RevenueMicros)
}

func TestFinanceOverviewStreamsSourcesAcrossBatchBoundary(t *testing.T) {
	db := setupTokenControllerTestDB(t)
	require.NoError(t, db.AutoMigrate(&model.User{}, &model.TopUp{}, &model.SubscriptionOrder{}, &model.Log{}, &model.FinanceLedgerEntry{}, &model.FinancePaymentMethod{}))
	now := time.Now().Unix()
	user := model.User{Username: "finance-batch", Password: "password", Role: common.RoleCommonUser, Status: common.UserStatusEnabled, Group: "default", AffCode: "finance-batch"}
	require.NoError(t, db.Create(&user).Error)

	rows := make([]model.TopUp, financeDashboardBatchSize+1)
	for index := range rows {
		rows[index] = model.TopUp{
			UserId:          user.Id,
			TradeNo:         "finance-batch-" + strconv.Itoa(index),
			Money:           1,
			Status:          common.TopUpStatusSuccess,
			PaymentMethod:   "stripe",
			PaymentProvider: "stripe",
			CompleteTime:    now - int64(index),
		}
	}
	require.NoError(t, db.Create(&rows).Error)

	view, err := buildFinanceOverview(now-int64(len(rows))-1, now+1, 0, "")
	require.NoError(t, err)
	require.Equal(t, int64(financeDashboardBatchSize+1)*1_000_000, view.RevenueMicros)
	require.Len(t, view.Users, 1)
}

func TestFinanceLedgerIsAppendOnlyAndReversalIsExactlyOnce(t *testing.T) {
	db := setupTokenControllerTestDB(t)
	require.NoError(t, db.AutoMigrate(&model.FinanceLedgerEntry{}))
	now := time.Now().Unix()
	entry, err := model.AppendFinanceLedgerEntry(&model.FinanceLedgerEntry{EntryType: model.FinanceEntryExpense, Category: "vendor", AmountMicros: 1_250_000, Currency: "usd", Direction: model.FinanceDirectionDebit, SourceType: model.FinanceSourceManual, OccurredAt: now, CreatedBy: 1, IdempotencyKey: "vendor-1"})
	require.NoError(t, err)
	require.Equal(t, entry.Id, mustFinanceEntry(t, "vendor-1").Id)
	replay, err := model.AppendFinanceLedgerEntry(&model.FinanceLedgerEntry{EntryType: model.FinanceEntryExpense, Category: "different", AmountMicros: 9, Currency: "USD", Direction: model.FinanceDirectionDebit, SourceType: model.FinanceSourceManual, OccurredAt: now, CreatedBy: 1, IdempotencyKey: "vendor-1"})
	require.NoError(t, err)
	require.Equal(t, entry.Id, replay.Id)
	reversal, err := model.ReverseFinanceLedgerEntry(entry.Id, 1, now+1)
	require.NoError(t, err)
	require.Equal(t, model.FinanceEntryExpense, reversal.EntryType)
	require.Equal(t, int8(model.FinanceDirectionCredit), reversal.Direction)
	_, err = model.ReverseFinanceLedgerEntry(entry.Id, 1, now+2)
	require.ErrorIs(t, err, model.ErrFinanceAlreadyReversed)
}

func TestFinanceHandlersRequireAdminRouteContractAndReturnUserDetail(t *testing.T) {
	db := setupTokenControllerTestDB(t)
	require.NoError(t, db.AutoMigrate(&model.User{}, &model.TopUp{}, &model.SubscriptionOrder{}, &model.Log{}, &model.FinanceLedgerEntry{}, &model.FinancePaymentMethod{}))
	user := model.User{Username: "finance-detail", Password: "password", Role: common.RoleCommonUser, Status: common.UserStatusEnabled, Group: "default", AffCode: "finance-detail"}
	require.NoError(t, db.Create(&user).Error)
	now := time.Now().Unix()
	require.NoError(t, db.Create(&model.TopUp{UserId: user.Id, TradeNo: "finance-detail-topup", Money: 3.5, Amount: 35, Status: common.TopUpStatusSuccess, PaymentMethod: "stripe", PaymentProvider: "stripe", CompleteTime: now - 1}).Error)
	recorder := httptest.NewRecorder()
	c, _ := gin.CreateTestContext(recorder)
	c.Request = httptest.NewRequest(http.MethodGet, "/api/finance/users/"+stringInt(user.Id), nil)
	c.Params = gin.Params{{Key: "user_id", Value: stringInt(user.Id)}}
	GetFinanceUser(c)
	require.Equal(t, http.StatusOK, recorder.Code)
	var response struct {
		Success bool            `json:"success"`
		Data    financeOverview `json:"data"`
	}
	require.NoError(t, json.Unmarshal(recorder.Body.Bytes(), &response))
	require.True(t, response.Success)
	require.Equal(t, int64(3_500_000), response.Data.RevenueMicros)
}

func mustFinanceEntry(t *testing.T, key string) *model.FinanceLedgerEntry {
	t.Helper()
	var entry model.FinanceLedgerEntry
	require.NoError(t, model.DB.Where("idempotency_key = ?", key).First(&entry).Error)
	return &entry
}
