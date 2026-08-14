package model

import (
	"testing"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/LIghtJUNction/api.lmm.best/setting/operation_setting"
	"github.com/stretchr/testify/require"
)

func TestApplyViolationFeeEscalatesResetsAndNeverOverdraws(t *testing.T) {
	db := setupConsoleActivationTestDB(t)
	require.NoError(t, db.AutoMigrate(&User{}, &ViolationFeeState{}, &ViolationFeeRecord{}, &ViolationFeeAppeal{}))
	user := &User{Username: "violation-fee-user", Quota: int(common.QuotaPerUnit * 1.25), Status: common.UserStatusEnabled}
	require.NoError(t, db.Create(user).Error)
	policy := operation_setting.ViolationFeePolicy{
		Groups: []string{"default"}, Enabled: true, AmountsUSD: []float64{0.5, 1},
		InitialAmountUSD: 0.5, Multiplier: 2, MaxAmountUSD: 8, PeriodSeconds: 100, DrainBalanceWhenShort: true,
	}

	first, err := ApplyViolationFee(ViolationFeeChargeInput{UserID: user.Id, RequestID: "violation-1", Policy: policy, Group: "default", Now: 1000})
	require.NoError(t, err)
	require.Equal(t, 1, first.Record.Occurrence)
	require.Equal(t, int(common.QuotaPerUnit*0.5), first.Record.ChargedQuota)

	second, err := ApplyViolationFee(ViolationFeeChargeInput{UserID: user.Id, RequestID: "violation-2", Policy: policy, Group: "default", Now: 1001})
	require.NoError(t, err)
	require.Equal(t, 2, second.Record.Occurrence)
	require.Equal(t, int(common.QuotaPerUnit*0.75), second.Record.ChargedQuota, "only remaining wallet quota is charged")

	var stored User
	require.NoError(t, db.First(&stored, user.Id).Error)
	require.Equal(t, 0, stored.Quota)

	reset, err := ApplyViolationFee(ViolationFeeChargeInput{UserID: user.Id, RequestID: "violation-3", Policy: policy, Group: "default", Now: 1100})
	require.NoError(t, err)
	require.Equal(t, 1, reset.Record.Occurrence, "counter resets at the period boundary")
	require.Equal(t, 0, reset.Record.ChargedQuota)
}

func TestApplyViolationFeeIsIdempotentAndAppealCanReverse(t *testing.T) {
	db := setupConsoleActivationTestDB(t)
	require.NoError(t, db.AutoMigrate(&User{}, &ViolationFeeState{}, &ViolationFeeRecord{}, &ViolationFeeAppeal{}))
	user := &User{Username: "violation-fee-appeal-user", Quota: int(common.QuotaPerUnit), Status: common.UserStatusEnabled}
	require.NoError(t, db.Create(user).Error)
	policy := operation_setting.ViolationFeePolicy{Groups: []string{"*"}, AmountsUSD: []float64{0.5}, InitialAmountUSD: 0.5, Multiplier: 2, MaxAmountUSD: 2, PeriodSeconds: 100, DrainBalanceWhenShort: true}

	first, err := ApplyViolationFee(ViolationFeeChargeInput{UserID: user.Id, RequestID: "same-request", Policy: policy, Group: "default", Now: 2000})
	require.NoError(t, err)
	retry, err := ApplyViolationFee(ViolationFeeChargeInput{UserID: user.Id, RequestID: "same-request", Policy: policy, Group: "default", Now: 2001})
	require.NoError(t, err)
	require.True(t, retry.AlreadyExist)
	require.Equal(t, first.Record.ID, retry.Record.ID)

	appeal, err := SubmitViolationFeeAppeal(user.Id, first.Record.ID, "这是误判，请复核")
	require.NoError(t, err)
	_, err = ReviewViolationFeeAppeal(99, appeal.ID, true, "确认误判，退回处罚")
	require.NoError(t, err)
	var record ViolationFeeRecord
	require.NoError(t, db.First(&record, first.Record.ID).Error)
	require.Equal(t, ViolationFeeRecordStatusReversed, record.Status)
	var stored User
	require.NoError(t, db.First(&stored, user.Id).Error)
	require.Equal(t, int(common.QuotaPerUnit), stored.Quota)
}
