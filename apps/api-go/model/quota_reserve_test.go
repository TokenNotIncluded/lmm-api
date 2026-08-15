package model

import (
	"testing"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/stretchr/testify/require"
)

func TestQuotaReserveFallsBackToConditionalDatabaseBalance(t *testing.T) {
	truncateTables(t)
	previousRedis := common.RedisEnabled
	common.RedisEnabled = false
	t.Cleanup(func() { common.RedisEnabled = previousRedis })

	user := User{
		Username: "quota-reserve-user",
		Password: "password",
		Role:     common.RoleCommonUser,
		Status:   common.UserStatusEnabled,
		Quota:    100,
	}
	require.NoError(t, DB.Create(&user).Error)

	reserved, err := TryReserveUserQuota(user.Id, 60)
	require.NoError(t, err)
	require.True(t, reserved)

	reserved, err = TryReserveUserQuota(user.Id, 50)
	require.NoError(t, err)
	require.False(t, reserved)

	var stored User
	require.NoError(t, DB.First(&stored, user.Id).Error)
	require.Equal(t, 40, stored.Quota)
}
