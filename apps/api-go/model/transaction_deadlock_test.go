package model

import (
	"context"
	"path/filepath"
	"testing"
	"time"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/LIghtJUNction/api.lmm.best/constant"
	"github.com/glebarez/sqlite"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
	"gorm.io/gorm"
)

func useSingleConnectionTestDB(t *testing.T, models ...any) {
	t.Helper()

	baseDB, baseLogDB := DB, LOG_DB
	db, err := gorm.Open(sqlite.Open(filepath.Join(t.TempDir(), "model.db")), &gorm.Config{})
	require.NoError(t, err)
	require.NoError(t, db.AutoMigrate(models...))
	sqlDB, err := db.DB()
	require.NoError(t, err)
	sqlDB.SetMaxOpenConns(1)
	DB, LOG_DB = db, db
	t.Cleanup(func() {
		DB, LOG_DB = baseDB, baseLogDB
		_ = sqlDB.Close()
	})
}

func withSingleConnectionDeadline(t *testing.T, run func() error) error {
	t.Helper()

	ctx, cancel := context.WithTimeout(context.Background(), 500*time.Millisecond)
	defer cancel()

	baseDB := DB
	DB = baseDB.WithContext(ctx)
	defer func() {
		DB = baseDB
	}()

	return run()
}

func TestRefundSubscriptionPreConsumeDoesNotDeadlockWithOneConnection(t *testing.T) {
	useSingleConnectionTestDB(t, &UserSubscription{}, &SubscriptionPreConsumeRecord{})

	subscription := UserSubscription{
		UserId:      1,
		PlanId:      1,
		AmountTotal: 100,
		AmountUsed:  60,
		Status:      "active",
	}
	require.NoError(t, DB.Create(&subscription).Error)
	record := SubscriptionPreConsumeRecord{
		RequestId:          "refund-single-connection",
		UserId:             subscription.UserId,
		UserSubscriptionId: subscription.Id,
		PreConsumed:        20,
		Status:             "consumed",
	}
	require.NoError(t, DB.Create(&record).Error)

	err := withSingleConnectionDeadline(t, func() error {
		return RefundSubscriptionPreConsume(record.RequestId)
	})
	require.NoError(t, err)

	require.NoError(t, DB.First(&subscription, subscription.Id).Error)
	assert.EqualValues(t, 40, subscription.AmountUsed)
	require.NoError(t, DB.First(&record, record.Id).Error)
	assert.Equal(t, "refunded", record.Status)
}

func TestBatchSetChannelTagDoesNotDeadlockWithOneConnection(t *testing.T) {
	useSingleConnectionTestDB(t, &Channel{}, &Ability{})

	oldTag := "old"
	channel := Channel{
		Type:   constant.ChannelTypeOpenAI,
		Key:    "test-key",
		Status: common.ChannelStatusEnabled,
		Name:   "single-connection-channel",
		Models: "gpt-test",
		Group:  "default",
		Tag:    &oldTag,
	}
	require.NoError(t, DB.Create(&channel).Error)
	require.NoError(t, channel.AddAbilities(nil))

	newTag := "new"
	err := withSingleConnectionDeadline(t, func() error {
		return BatchSetChannelTag([]int{channel.Id}, &newTag)
	})
	require.NoError(t, err)

	require.NoError(t, DB.First(&channel, channel.Id).Error)
	require.NotNil(t, channel.Tag)
	assert.Equal(t, newTag, *channel.Tag)
	var ability Ability
	require.NoError(t, DB.First(&ability, "channel_id = ?", channel.Id).Error)
	require.NotNil(t, ability.Tag)
	assert.Equal(t, newTag, *ability.Tag)
}
