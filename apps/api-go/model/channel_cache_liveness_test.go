package model

import (
	"testing"
	"time"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/LIghtJUNction/api.lmm.best/relaykit/dto"
	"github.com/glebarez/sqlite"
	"gorm.io/gorm"
)

func TestInvalidatePricingCacheDoesNotWaitForPricingRefresh(t *testing.T) {
	updatePricingLock.Lock()
	done := make(chan struct{})
	go func() {
		InvalidatePricingCache()
		close(done)
	}()

	timedOut := false
	select {
	case <-done:
	case <-time.After(250 * time.Millisecond):
		timedOut = true
	}
	updatePricingLock.Unlock()

	if timedOut {
		<-done
		t.Fatal("pricing invalidation blocked behind an in-progress pricing refresh")
	}
}

func TestInitChannelCacheKeepsLastKnownGoodOnDatabaseError(t *testing.T) {
	db, err := gorm.Open(sqlite.Open(":memory:"), &gorm.Config{})
	if err != nil {
		t.Fatalf("open test database: %v", err)
	}
	sqlDB, err := db.DB()
	if err != nil {
		t.Fatalf("get test database handle: %v", err)
	}
	if err := sqlDB.Close(); err != nil {
		t.Fatalf("close test database: %v", err)
	}

	previousDB := DB
	previousMemoryCacheEnabled := common.MemoryCacheEnabled
	channelSyncLock.Lock()
	previousGroups := group2model2channels
	previousChannels := channelsIDM
	previousAdvancedConfigs := channel2advancedCustomConfig
	previousReady := channelCacheReady
	previousLastError := channelCacheLastError
	sentinel := &Channel{Id: 42, Name: "last-known-good"}
	group2model2channels = map[string]map[string][]int{
		"default": {"gpt-image-2": {42}},
	}
	channelsIDM = map[int]*Channel{42: sentinel}
	channel2advancedCustomConfig = make(map[int]*dto.AdvancedCustomConfig)
	channelCacheReady = true
	channelCacheLastError = nil
	channelSyncLock.Unlock()
	DB = db
	common.MemoryCacheEnabled = true

	t.Cleanup(func() {
		DB = previousDB
		common.MemoryCacheEnabled = previousMemoryCacheEnabled
		channelSyncLock.Lock()
		group2model2channels = previousGroups
		channelsIDM = previousChannels
		channel2advancedCustomConfig = previousAdvancedConfigs
		channelCacheReady = previousReady
		channelCacheLastError = previousLastError
		channelSyncLock.Unlock()
	})

	InitChannelCache()

	channelSyncLock.RLock()
	defer channelSyncLock.RUnlock()
	if channelsIDM[42] != sentinel {
		t.Fatal("failed refresh replaced the last-known-good channel cache")
	}
	channels := group2model2channels["default"]["gpt-image-2"]
	if len(channels) != 1 || channels[0] != 42 {
		t.Fatalf("failed refresh changed channel selection: %v", channels)
	}
}
