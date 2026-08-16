// Copyright (C) 2026 LIghtJUNction
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as
// published by the Free Software Foundation, either version 3 of the
// License, or (at your option) any later version.

package model

import (
	"testing"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestApplyWaffoPancakeRefundAccumulatesSubscriptionPartialRefunds(t *testing.T) {
	db := setupConsoleActivationTestDB(t)
	require.NoError(t, db.AutoMigrate(&SubscriptionOrder{}, &UserSubscription{}, &FinanceLedgerEntry{}))

	user := User{Username: "refund-subscription-owner", Password: "password", Status: common.UserStatusEnabled}
	require.NoError(t, db.Create(&user).Error)
	subscription := UserSubscription{
		UserId: user.Id, AmountTotal: 100_000, AmountUsed: 0,
		Status: "active", Source: "order", EndTime: common.GetTimestamp() + 3600,
	}
	require.NoError(t, db.Create(&subscription).Error)
	order := SubscriptionOrder{
		UserId: user.Id, TradeNo: "refund-subscription-partial",
		PaymentProvider: PaymentProviderWaffoPancake, PaymentMethod: PaymentMethodWaffoPancake,
		UserSubscriptionId: subscription.Id, Money: 10,
		Status: common.TopUpStatusSuccess,
	}
	require.NoError(t, db.Create(&order).Error)

	for _, eventID := range []string{"subscription-refund-event-1", "subscription-refund-event-2"} {
		result, err := ApplyWaffoPancakeRefund(
			order.TradeNo, true, 2_500_000, FinanceCurrencyUSD,
			eventID,
			PaymentMethodWaffoPancake, PaymentProviderWaffoPancake, "test refund", user.Id,
		)
		require.NoError(t, err)
		assert.Equal(t, int64(25_000), result.QuotaDebited)
	}

	var refreshedSubscription UserSubscription
	require.NoError(t, db.First(&refreshedSubscription, subscription.Id).Error)
	assert.Equal(t, int64(50_000), refreshedSubscription.AmountTotal)

	var refreshedOrder SubscriptionOrder
	require.NoError(t, db.First(&refreshedOrder, order.Id).Error)
	assert.Equal(t, int64(5_000_000), refreshedOrder.RefundedAmountMicros)
	assert.Equal(t, int64(50_000), refreshedOrder.RefundedQuota)

	var refreshedUser User
	require.NoError(t, db.First(&refreshedUser, user.Id).Error)
	assert.Zero(t, refreshedUser.Quota, "subscription refunds must not debit the wallet quota")
}

func TestApplyWaffoPancakeRefundRejectsMissingWalletOrder(t *testing.T) {
	db := setupConsoleActivationTestDB(t)
	require.NoError(t, db.AutoMigrate(&TopUp{}, &FinanceLedgerEntry{}))

	_, err := ApplyWaffoPancakeRefund(
		"missing-wallet-order", false, 1_000_000, FinanceCurrencyUSD,
		"missing-wallet-refund-event", PaymentMethodWaffoPancake,
		PaymentProviderWaffoPancake, "test refund", 1,
	)
	require.Error(t, err)
	assert.ErrorContains(t, err, "refund order disappeared")

	var entries []FinanceLedgerEntry
	require.NoError(t, db.Find(&entries).Error)
	assert.Empty(t, entries)
}
