package controller

import (
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/LIghtJUNction/api.lmm.best/model"
	"github.com/LIghtJUNction/api.lmm.best/service"
	"github.com/gin-gonic/gin"
	"github.com/stretchr/testify/require"
)

func TestHandleWaffoPancakeRefundEventRecordsUserVisibleRefundLog(t *testing.T) {
	tests := []struct {
		name      string
		eventType string
		amount    string
		wantText  string
	}{
		{name: "succeeded", eventType: "refund.succeeded", amount: "2.50", wantText: "refund.succeeded"},
		{name: "failed", eventType: "refund.failed", amount: "", wantText: "refund.failed"},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			db := setupTokenControllerTestDB(t)
			require.NoError(t, db.AutoMigrate(&model.TopUp{}, &model.Log{}, &model.FinanceLedgerEntry{}))
			user := model.User{
				Username: "pancake-refund-" + tt.name,
				Password: "password",
				Status:   common.UserStatusEnabled,
			}
			require.NoError(t, db.Create(&user).Error)
			require.NoError(t, db.Create(&model.TopUp{
				UserId:          user.Id,
				TradeNo:         "WAFFO_PANCAKE-" + tt.name,
				PaymentMethod:   model.PaymentMethodWaffoPancake,
				PaymentProvider: model.PaymentProviderWaffoPancake,
				Status:          common.TopUpStatusSuccess,
				Money:           2.50,
			}).Error)

			ctx, _ := gin.CreateTestContext(httptest.NewRecorder())
			ctx.Request = httptest.NewRequest(http.MethodPost, "/", nil)
			event := &service.WaffoPancakeWebhookEvent{
				ID:        "evt-refund-" + tt.name,
				EventType: tt.eventType,
				Data: service.WaffoPancakeWebhookData{
					OrderMerchantExternalID:        "WAFFO_PANCAKE-" + tt.name,
					RefundTicketMerchantExternalID: "refund-" + tt.name,
					Amount:                         tt.amount,
					Currency:                       "USD",
					RefundReason:                   "provider declined",
				},
			}

			require.NoError(t, handleWaffoPancakeRefundEvent(ctx, event))
			var logs []model.Log
			require.NoError(t, db.Where("user_id = ?", user.Id).Find(&logs).Error)
			require.Len(t, logs, 1)
			require.Equal(t, model.LogTypeRefund, logs[0].Type)
			require.Contains(t, logs[0].Content, tt.wantText)
		})
	}
}

func TestHandleWaffoPancakeSubscriptionRefundIsLedgerIdempotent(t *testing.T) {
	db := setupTokenControllerTestDB(t)
	require.NoError(t, db.AutoMigrate(&model.User{}, &model.SubscriptionOrder{}, &model.Log{}, &model.FinanceLedgerEntry{}))
	user := model.User{
		Username: "pancake-subscription-refund",
		Password: "password",
		Status:   common.UserStatusEnabled,
	}
	require.NoError(t, db.Create(&user).Error)
	tradeNo := "WAFFO_PANCAKE_SUB-1-refund"
	require.NoError(t, db.Create(&model.SubscriptionOrder{
		UserId:          user.Id,
		TradeNo:         tradeNo,
		PaymentMethod:   model.PaymentMethodWaffoPancake,
		PaymentProvider: model.PaymentProviderWaffoPancake,
		Status:          common.TopUpStatusSuccess,
		Money:           2.50,
	}).Error)

	ctx, _ := gin.CreateTestContext(httptest.NewRecorder())
	ctx.Request = httptest.NewRequest(http.MethodPost, "/", nil)
	event := &service.WaffoPancakeWebhookEvent{
		ID:        "evt-subscription-refund",
		EventType: "refund.succeeded",
		Data: service.WaffoPancakeWebhookData{
			OrderMerchantExternalID:        tradeNo,
			RefundTicketMerchantExternalID: "refund-subscription-1",
			Amount:                         "2.50",
			Currency:                       "USD",
		},
	}

	require.NoError(t, handleWaffoPancakeRefundEvent(ctx, event))
	// Providers retry webhook deliveries. The second delivery must not create
	// a second financial row or duplicate the user-visible audit log.
	require.NoError(t, handleWaffoPancakeRefundEvent(ctx, event))

	var entries []model.FinanceLedgerEntry
	require.NoError(t, db.Where("source_type = ? AND source_id = ?", model.FinanceSourceRefund, event.ID).Find(&entries).Error)
	require.Len(t, entries, 1)
	require.Equal(t, model.PaymentProviderWaffoPancake, entries[0].PaymentProvider)
	require.Equal(t, int64(2_500_000), entries[0].AmountMicros)

	var logs []model.Log
	require.NoError(t, db.Where("user_id = ? AND type = ?", user.Id, model.LogTypeRefund).Find(&logs).Error)
	require.Len(t, logs, 1)
}
