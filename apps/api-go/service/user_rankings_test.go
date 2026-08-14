package service

import (
	"testing"
	"time"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/LIghtJUNction/api.lmm.best/model"
	"github.com/LIghtJUNction/api.lmm.best/relaykit/dto"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestGetUserUsageRankingsSnapshotHonorsVisibility(t *testing.T) {
	require.NoError(t, model.DB.AutoMigrate(&model.QuotaData{}))
	t.Cleanup(func() {
		model.DB.Exec("DELETE FROM quota_data")
		model.DB.Exec("DELETE FROM users")
	})

	anonymous := model.User{
		Id:       101,
		Username: "anonymous-user",
		AffCode:  "anonymous-user-aff",
		Status:   common.UserStatusEnabled,
	}
	public := model.User{
		Id:          102,
		Username:    "public-user",
		AffCode:     "public-user-aff",
		DisplayName: "Public user",
		Status:      common.UserStatusEnabled,
	}
	public.SetSetting(dto.UserSetting{
		UsageLeaderboardVisibility: dto.UsageLeaderboardVisibilityPublic,
	})
	hidden := model.User{
		Id:       103,
		Username: "hidden-user",
		AffCode:  "hidden-user-aff",
		Status:   common.UserStatusEnabled,
	}
	hidden.SetSetting(dto.UserSetting{
		UsageLeaderboardVisibility: dto.UsageLeaderboardVisibilityHidden,
	})
	disabled := model.User{
		Id:       104,
		Username: "disabled-user",
		AffCode:  "disabled-user-aff",
		Status:   common.UserStatusDisabled,
	}
	anonymousTwo := model.User{
		Id:       105,
		Username: "anonymous-user-two",
		AffCode:  "anonymous-user-two-aff",
		Status:   common.UserStatusEnabled,
	}

	require.NoError(t, model.DB.Create(&[]model.User{anonymous, public, hidden, disabled, anonymousTwo}).Error)
	createdAt := time.Now().Add(-time.Hour).Unix()
	require.NoError(t, model.DB.Create(&[]model.QuotaData{
		{UserID: anonymous.Id, CreatedAt: createdAt, Count: 2, TokenUsed: 100},
		{UserID: public.Id, CreatedAt: createdAt, Count: 4, TokenUsed: 300},
		{UserID: hidden.Id, CreatedAt: createdAt, Count: 10, TokenUsed: 1000},
		{UserID: disabled.Id, CreatedAt: createdAt, Count: 8, TokenUsed: 500},
		{UserID: 105, CreatedAt: createdAt, Count: 1, TokenUsed: 50},
	}).Error)

	result, err := GetUserUsageRankingsSnapshot("week")
	require.NoError(t, err)
	assert.Equal(t, "week", result.Period)
	assert.Equal(t, int64(450), result.TotalTokens)
	assert.Equal(t, int64(7), result.TotalRequests)
	assert.Equal(t, 3, result.ParticipantCount)
	assert.Equal(t, 2, result.AnonymousParticipantCount)
	require.Len(t, result.Users, 3)

	assert.Equal(t, 1, result.Users[0].Rank)
	assert.Equal(t, "Public user", result.Users[0].Name)
	assert.False(t, result.Users[0].Anonymous)
	assert.Equal(t, int64(300), result.Users[0].TotalTokens)
	assert.Equal(t, int64(4), result.Users[0].Requests)
	assert.InDelta(t, 2.0/3.0, result.Users[0].Share, 0.0001)

	assert.Equal(t, 2, result.Users[1].Rank)
	assert.Empty(t, result.Users[1].Name)
	assert.True(t, result.Users[1].Anonymous)
	assert.Equal(t, int64(100), result.Users[1].TotalTokens)
	assert.Equal(t, int64(2), result.Users[1].Requests)
	assert.InDelta(t, 2.0/9.0, result.Users[1].Share, 0.0001)

	assert.Equal(t, 3, result.Users[2].Rank)
	assert.Empty(t, result.Users[2].Name)
	assert.True(t, result.Users[2].Anonymous)
	assert.Equal(t, int64(50), result.Users[2].TotalTokens)
	assert.Equal(t, int64(1), result.Users[2].Requests)
	assert.InDelta(t, 1.0/9.0, result.Users[2].Share, 0.0001)
}
