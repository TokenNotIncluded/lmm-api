package model

import (
	"context"
	"errors"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/QuantumNous/new-api/common"
	"github.com/QuantumNous/new-api/constant"
	"github.com/QuantumNous/new-api/relaykit/dto"
	"github.com/glebarez/sqlite"
	"gorm.io/gorm"
)

func openCacheTestDB(t *testing.T, models ...interface{}) *gorm.DB {
	t.Helper()
	db, err := gorm.Open(sqlite.Open(":memory:"), &gorm.Config{})
	if err != nil {
		t.Fatalf("open cache test database: %v", err)
	}
	sqlDB, err := db.DB()
	if err != nil {
		t.Fatalf("get cache test database handle: %v", err)
	}
	sqlDB.SetMaxOpenConns(1)
	if len(models) > 0 {
		if err := db.AutoMigrate(models...); err != nil {
			t.Fatalf("migrate cache test database: %v", err)
		}
	}
	return db
}

func preservePricingTestState(t *testing.T) {
	t.Helper()
	previousDB := DB
	previousMemoryCacheEnabled := common.MemoryCacheEnabled
	previousSnapshot := pricingCache.Load()
	previousInvalidation := pricingInvalidation.Load()
	previousHook := pricingRefreshHook
	previousContextHook := pricingContextHook
	previousVendorHook := pricingVendorHook
	t.Cleanup(func() {
		updatePricingLock.Lock()
		defer updatePricingLock.Unlock()
		DB = previousDB
		common.MemoryCacheEnabled = previousMemoryCacheEnabled
		pricingRefreshHook = previousHook
		pricingContextHook = previousContextHook
		pricingVendorHook = previousVendorHook
		pricingInvalidation.Store(previousInvalidation)
		pricingCache.Store(previousSnapshot)
	})
}

func preserveChannelTestState(t *testing.T) {
	t.Helper()
	previousDB := DB
	previousMemoryCacheEnabled := common.MemoryCacheEnabled
	previousHook := channelRefreshHook
	previousAfterQueryHook := channelAfterChannelsQueryHook
	previousContextHook := channelContextHook
	channelSyncLock.Lock()
	previousGroups := group2model2channels
	previousChannels := channelsIDM
	previousAdvancedConfigs := channel2advancedCustomConfig
	previousReady := channelCacheReady
	previousLastError := channelCacheLastError
	channelSyncLock.Unlock()
	t.Cleanup(func() {
		channelRefreshLock.Lock()
		defer channelRefreshLock.Unlock()
		DB = previousDB
		common.MemoryCacheEnabled = previousMemoryCacheEnabled
		channelRefreshHook = previousHook
		channelAfterChannelsQueryHook = previousAfterQueryHook
		channelContextHook = previousContextHook
		channelSyncLock.Lock()
		group2model2channels = previousGroups
		channelsIDM = previousChannels
		channel2advancedCustomConfig = previousAdvancedConfigs
		channelCacheReady = previousReady
		channelCacheLastError = previousLastError
		channelSyncLock.Unlock()
	})
}

func preserveCacheWarmTestState(t *testing.T) {
	t.Helper()
	cacheWarmLock.Lock()
	previousEnforced := cacheReadinessEnforced.Load()
	cacheWarmStateLock.Lock()
	previousLastError := cacheWarmLastError
	previousNextRetry := cacheWarmNextRetry
	previousRetryDelay := cacheWarmRetryDelay
	previousNow := cacheWarmNow
	previousAttemptHook := cacheWarmAttemptHook
	cacheWarmStateLock.Unlock()
	cacheWarmLock.Unlock()
	t.Cleanup(func() {
		cacheWarmLock.Lock()
		defer cacheWarmLock.Unlock()
		cacheReadinessEnforced.Store(previousEnforced)
		cacheWarmStateLock.Lock()
		cacheWarmLastError = previousLastError
		cacheWarmNextRetry = previousNextRetry
		cacheWarmRetryDelay = previousRetryDelay
		cacheWarmNow = previousNow
		cacheWarmAttemptHook = previousAttemptHook
		cacheWarmStateLock.Unlock()
	})
}

func waitForCacheWarmIdle(t *testing.T) {
	t.Helper()
	deadline := time.Now().Add(time.Second)
	for time.Now().Before(deadline) {
		if cacheWarmLock.TryLock() {
			cacheWarmLock.Unlock()
			return
		}
		time.Sleep(time.Millisecond)
	}
	t.Fatal("cache warm attempt did not finish")
}

func waitForPricingRefreshIdle(t *testing.T) {
	t.Helper()
	deadline := time.Now().Add(time.Second)
	for time.Now().Before(deadline) {
		if updatePricingLock.TryLock() {
			updatePricingLock.Unlock()
			return
		}
		time.Sleep(time.Millisecond)
	}
	t.Fatal("pricing refresh attempt did not finish")
}

func TestConcurrentColdPricingCallersReturnImmediatelyAndStartOneRefresh(t *testing.T) {
	preservePricingTestState(t)
	DB = openCacheTestDB(t, &Channel{}, &Ability{}, &Model{}, &Vendor{})
	common.MemoryCacheEnabled = false
	pricingCache.Store(nil)
	pricingInvalidation.Store(50)

	started := make(chan struct{})
	release := make(chan struct{})
	var releaseOnce sync.Once
	t.Cleanup(func() { releaseOnce.Do(func() { close(release) }) })
	var refreshCount atomic.Int32
	pricingRefreshHook = func() {
		if refreshCount.Add(1) == 1 {
			close(started)
		}
		<-release
	}

	const callers = 128
	results := make(chan []Pricing, callers)
	for i := 0; i < callers; i++ {
		go func() { results <- GetPricing() }()
	}
	select {
	case <-started:
	case <-time.After(time.Second):
		t.Fatal("cold pricing refresh did not start")
	}
	for i := 0; i < callers; i++ {
		select {
		case result := <-results:
			if result != nil {
				t.Fatalf("cold caller received unexpected pricing: %#v", result)
			}
		case <-time.After(250 * time.Millisecond):
			t.Fatal("cold pricing caller blocked or queued behind refresh")
		}
	}
	if count := refreshCount.Load(); count != 1 {
		t.Fatalf("cold callers started %d refreshes, want 1", count)
	}
	releaseOnce.Do(func() { close(release) })
	deadline := time.After(time.Second)
	for pricingCache.Load() == nil {
		select {
		case <-deadline:
			t.Fatal("background pricing refresh did not publish")
		default:
			time.Sleep(time.Millisecond)
		}
	}
}

func TestPricingVendorWriteHonorsCanceledRefreshContext(t *testing.T) {
	preservePricingTestState(t)
	db := openCacheTestDB(t, &Channel{}, &Ability{}, &Model{}, &Vendor{})
	DB = db
	common.MemoryCacheEnabled = false
	channel := &Channel{Id: 701, Type: constant.ChannelTypeOpenAI, Status: common.ChannelStatusEnabled}
	if err := db.Create(channel).Error; err != nil {
		t.Fatalf("insert channel: %v", err)
	}
	if err := db.Create(&Ability{Group: "default", Model: "gpt-context-test", ChannelId: 701, Enabled: true}).Error; err != nil {
		t.Fatalf("insert ability: %v", err)
	}
	ctx, cancel := context.WithCancel(context.Background())
	pricingContextHook = func() (context.Context, context.CancelFunc) { return ctx, func() {} }
	pricingVendorHook = cancel
	pricingCache.Store(nil)

	err := refreshPricingNow()
	if err == nil || !errors.Is(err, context.Canceled) {
		t.Fatalf("vendor write error = %v, want context canceled", err)
	}
	if pricingCache.Load() != nil {
		t.Fatal("canceled vendor write published pricing snapshot")
	}
}

func TestSecondChannelQueryHonorsCanceledRefreshContext(t *testing.T) {
	preserveChannelTestState(t)
	DB = openCacheTestDB(t, &Channel{}, &Ability{})
	common.MemoryCacheEnabled = true
	ctx, cancel := context.WithCancel(context.Background())
	channelContextHook = func() (context.Context, context.CancelFunc) { return ctx, func() {} }
	channelAfterChannelsQueryHook = cancel

	err := refreshChannelCache()
	if err == nil || !errors.Is(err, context.Canceled) {
		t.Fatalf("second channel query error = %v, want context canceled", err)
	}
}

func TestPricingPartialQueryFailureKeepsCompleteSnapshot(t *testing.T) {
	preservePricingTestState(t)
	db := openCacheTestDB(t, &Channel{}, &Ability{}, &Model{})
	DB = db
	common.MemoryCacheEnabled = true

	before := &pricingSnapshot{
		pricing: []Pricing{{ModelName: "last-known-good", EnableGroup: []string{"default"}}},
		vendors: []PricingVendor{{ID: 7, Name: "sentinel"}},
		supportedEndpoints: map[string]common.EndpointInfo{
			"sentinel": {Path: "/sentinel", Method: "POST"},
		},
		modelEnableGroups: map[string][]string{
			"last-known-good": {"default"},
		},
		modelQuotaTypes: map[string]int{"last-known-good": 1},
		modelSupportEndpointTypes: map[string][]constant.EndpointType{
			"last-known-good": {constant.EndpointTypeOpenAI},
		},
		refreshedAt: time.Now(),
		generation:  10,
	}
	pricingCache.Store(before)
	pricingInvalidation.Store(11)

	if err := refreshPricingNow(); err == nil {
		t.Fatal("pricing refresh succeeded without the required vendors table")
	}
	if after := pricingCache.Load(); after != before {
		t.Fatal("failed pricing refresh replaced the complete last-known-good snapshot")
	}
}

func TestRefreshPricingReturnsSynchronousBuildError(t *testing.T) {
	preservePricingTestState(t)
	DB = openCacheTestDB(t, &Channel{}, &Ability{}, &Model{})
	common.MemoryCacheEnabled = false
	pricingCache.Store(nil)
	if err := RefreshPricing(); err == nil {
		t.Fatal("synchronous pricing refresh hid its build error")
	}
}

func TestWarmCachesConvertsPanicToReadinessError(t *testing.T) {
	preservePricingTestState(t)
	preserveCacheWarmTestState(t)
	DB = openCacheTestDB(t, &Channel{}, &Ability{}, &Model{}, &Vendor{})
	common.MemoryCacheEnabled = false
	pricingCache.Store(nil)
	pricingRefreshHook = func() { panic("deterministic warm panic") }

	err := WarmCaches()
	if err == nil || !strings.Contains(err.Error(), "deterministic warm panic") {
		t.Fatalf("warm error = %v, want recovered panic", err)
	}
	if readyErr := CacheReadinessError(); readyErr == nil || !strings.Contains(readyErr.Error(), "deterministic warm panic") {
		t.Fatalf("readiness error = %v, want recovered panic", readyErr)
	}
	if CachesReady() {
		t.Fatal("cache remained ready after warm panic")
	}
}

func TestAsyncPricingRefreshRecoversPanicAndReleasesSingleflight(t *testing.T) {
	preservePricingTestState(t)
	DB = openCacheTestDB(t, &Channel{}, &Ability{}, &Model{}, &Vendor{})
	common.MemoryCacheEnabled = false
	pricingCache.Store(nil)
	pricingRefreshHook = func() { panic("deterministic async pricing panic") }

	if pricing := GetPricing(); pricing != nil {
		t.Fatalf("cold pricing = %#v, want nil", pricing)
	}
	waitForPricingRefreshIdle(t)
	pricingRefreshHook = nil
	if err := RefreshPricing(); err != nil {
		t.Fatalf("refresh after recovered panic: %v", err)
	}
	if pricingCache.Load() == nil {
		t.Fatal("refresh lock remained stuck after recovered panic")
	}
}

func TestEnsureCachesWarmAsyncConvertsPanicAndRecovers(t *testing.T) {
	preservePricingTestState(t)
	preserveCacheWarmTestState(t)
	DB = openCacheTestDB(t, &Channel{}, &Ability{}, &Model{}, &Vendor{})
	common.MemoryCacheEnabled = false
	pricingCache.Store(nil)

	var nowNanos atomic.Int64
	start := time.Unix(1_700_000_100, 0)
	nowNanos.Store(start.UnixNano())
	cacheWarmNow = func() time.Time { return time.Unix(0, nowNanos.Load()) }
	pricingRefreshHook = func() { panic("deterministic async warm panic") }

	EnsureCachesWarmAsync()
	waitForCacheWarmIdle(t)
	if err := CacheReadinessError(); err == nil || !strings.Contains(err.Error(), "deterministic async warm panic") {
		t.Fatalf("readiness error = %v, want recovered async panic", err)
	}

	pricingRefreshHook = nil
	nowNanos.Add(cacheWarmRetryInitialDelay.Nanoseconds())
	EnsureCachesWarmAsync()
	waitForCacheWarmIdle(t)
	if err := CacheReadinessError(); err != nil {
		t.Fatalf("readiness did not recover after async panic: %v", err)
	}
}

func TestEnsureCachesWarmAsyncBacksOffAndRecovers(t *testing.T) {
	preservePricingTestState(t)
	preserveCacheWarmTestState(t)
	DB = openCacheTestDB(t, &Channel{}, &Ability{}, &Model{})
	common.MemoryCacheEnabled = false
	pricingCache.Store(nil)

	var nowNanos atomic.Int64
	start := time.Unix(1_700_000_000, 0)
	nowNanos.Store(start.UnixNano())
	cacheWarmNow = func() time.Time { return time.Unix(0, nowNanos.Load()) }
	var attempts atomic.Int32
	cacheWarmAttemptHook = func() { attempts.Add(1) }

	EnsureCachesWarmAsync()
	waitForCacheWarmIdle(t)
	if got := attempts.Load(); got != 1 {
		t.Fatalf("initial attempts = %d, want 1", got)
	}

	for i := 0; i < 64; i++ {
		EnsureCachesWarmAsync()
	}
	if got := attempts.Load(); got != 1 {
		t.Fatalf("probes bypassed cooldown: attempts = %d, want 1", got)
	}

	nowNanos.Add((cacheWarmRetryInitialDelay - time.Millisecond).Nanoseconds())
	EnsureCachesWarmAsync()
	if got := attempts.Load(); got != 1 {
		t.Fatalf("retry ran before initial cooldown: attempts = %d, want 1", got)
	}
	nowNanos.Add(time.Millisecond.Nanoseconds())
	EnsureCachesWarmAsync()
	waitForCacheWarmIdle(t)
	if got := attempts.Load(); got != 2 {
		t.Fatalf("attempts after initial cooldown = %d, want 2", got)
	}

	DB = openCacheTestDB(t, &Channel{}, &Ability{}, &Model{}, &Vendor{})
	nowNanos.Add((2*cacheWarmRetryInitialDelay - time.Millisecond).Nanoseconds())
	EnsureCachesWarmAsync()
	if got := attempts.Load(); got != 2 {
		t.Fatalf("retry ran before doubled cooldown: attempts = %d, want 2", got)
	}
	nowNanos.Add(time.Millisecond.Nanoseconds())
	EnsureCachesWarmAsync()
	waitForCacheWarmIdle(t)
	if got := attempts.Load(); got != 3 {
		t.Fatalf("recovery attempts = %d, want 3", got)
	}
	if err := CacheReadinessError(); err != nil {
		t.Fatalf("readiness did not recover: %v", err)
	}
	if !CachesReady() {
		t.Fatal("cache did not become ready after successful retry")
	}
}

func TestPricingInvalidationDuringRefreshRemainsPending(t *testing.T) {
	preservePricingTestState(t)
	DB = openCacheTestDB(t, &Channel{}, &Ability{}, &Model{}, &Vendor{})
	common.MemoryCacheEnabled = true
	pricingCache.Store(nil)
	pricingInvalidation.Store(20)
	pricingRefreshHook = func() {
		InvalidatePricingCache()
		pricingRefreshHook = nil
	}

	if err := refreshPricingNow(); err != nil {
		t.Fatalf("refresh pricing: %v", err)
	}
	snapshot := pricingCache.Load()
	if snapshot == nil {
		t.Fatal("successful refresh did not publish a snapshot")
	}
	if snapshot.generation != 20 {
		t.Fatalf("published generation = %d, want 20", snapshot.generation)
	}
	if pricingInvalidation.Load() != 21 {
		t.Fatalf("requested generation = %d, want 21", pricingInvalidation.Load())
	}
	if !pricingCacheNeedsRefresh(snapshot) {
		t.Fatal("invalidation racing with refresh was lost")
	}
}

func TestGetPricingServesLastKnownGoodWhileRefreshIsBusy(t *testing.T) {
	preservePricingTestState(t)
	snapshot := &pricingSnapshot{
		pricing:     []Pricing{{ModelName: "last-known-good"}},
		refreshedAt: time.Now(),
		generation:  40,
	}
	pricingCache.Store(snapshot)
	pricingInvalidation.Store(41)
	updatePricingLock.Lock()
	done := make(chan []Pricing, 1)
	go func() {
		done <- GetPricing()
	}()

	select {
	case pricing := <-done:
		updatePricingLock.Unlock()
		if len(pricing) != 1 || pricing[0].ModelName != "last-known-good" {
			t.Fatalf("busy refresh did not serve last-known-good pricing: %#v", pricing)
		}
	case <-time.After(250 * time.Millisecond):
		updatePricingLock.Unlock()
		<-done
		t.Fatal("pricing request waited behind database refresh serialization")
	}
}

func TestPricingGettersReturnDefensiveCopies(t *testing.T) {
	preservePricingTestState(t)
	cacheRatio := 0.5
	snapshot := &pricingSnapshot{
		pricing: []Pricing{{
			ModelName:              "safe-model",
			CacheRatio:             &cacheRatio,
			EnableGroup:            []string{"default"},
			SupportedEndpointTypes: []constant.EndpointType{constant.EndpointTypeOpenAI},
		}},
		vendors: []PricingVendor{{ID: 1, Name: "safe-vendor"}},
		supportedEndpoints: map[string]common.EndpointInfo{
			"openai": {Path: "/v1/chat/completions", Method: "POST"},
		},
		modelEnableGroups: map[string][]string{"safe-model": {"default"}},
		modelQuotaTypes:   map[string]int{"safe-model": 1},
		modelSupportEndpointTypes: map[string][]constant.EndpointType{
			"safe-model": {constant.EndpointTypeOpenAI},
		},
		refreshedAt: time.Now(),
		generation:  30,
	}
	pricingInvalidation.Store(30)
	pricingCache.Store(snapshot)

	pricing := GetPricing()
	pricing[0].ModelName = "mutated"
	pricing[0].EnableGroup[0] = "mutated"
	pricing[0].SupportedEndpointTypes[0] = constant.EndpointTypeGemini
	*pricing[0].CacheRatio = 9
	vendors := GetVendors()
	vendors[0].Name = "mutated"
	endpoints := GetSupportedEndpointMap()
	endpoints["openai"] = common.EndpointInfo{Path: "/mutated", Method: "GET"}
	groups := GetModelEnableGroups("safe-model")
	groups[0] = "mutated"
	supported := GetModelSupportEndpointTypes("safe-model")
	supported[0] = constant.EndpointTypeGemini
	quota := GetModelQuotaTypes("safe-model")
	quota[0] = 9

	if snapshot.pricing[0].ModelName != "safe-model" || snapshot.pricing[0].EnableGroup[0] != "default" {
		t.Fatal("pricing getter exposed mutable snapshot storage")
	}
	if *snapshot.pricing[0].CacheRatio != 0.5 {
		t.Fatal("pricing getter exposed ratio pointer storage")
	}
	if snapshot.vendors[0].Name != "safe-vendor" {
		t.Fatal("vendor getter exposed snapshot storage")
	}
	if snapshot.supportedEndpoints["openai"].Path != "/v1/chat/completions" {
		t.Fatal("supported endpoint getter exposed snapshot map")
	}
	if snapshot.modelEnableGroups["safe-model"][0] != "default" ||
		snapshot.modelSupportEndpointTypes["safe-model"][0] != constant.EndpointTypeOpenAI ||
		snapshot.modelQuotaTypes["safe-model"] != 1 {
		t.Fatal("derived pricing getter exposed snapshot storage")
	}

	var workers sync.WaitGroup
	for i := 0; i < 16; i++ {
		workers.Add(1)
		go func() {
			defer workers.Done()
			for j := 0; j < 100; j++ {
				copy := GetPricing()
				copy[0].EnableGroup[0] = "caller-local"
				endpointCopy := GetSupportedEndpointMap()
				delete(endpointCopy, "openai")
			}
		}()
	}
	workers.Wait()
	if snapshot.pricing[0].EnableGroup[0] != "default" || len(snapshot.supportedEndpoints) != 1 {
		t.Fatal("concurrent getters mutated the immutable snapshot")
	}
}

func TestConcurrentChannelRefreshCannotPublishOlderResultLast(t *testing.T) {
	preserveChannelTestState(t)
	db := openCacheTestDB(t, &Channel{}, &Ability{})
	DB = db
	common.MemoryCacheEnabled = true
	channel := &Channel{
		Id:     501,
		Name:   "older",
		Status: common.ChannelStatusEnabled,
		Group:  "default",
		Models: "cache-model",
	}
	if err := db.Create(channel).Error; err != nil {
		t.Fatalf("insert channel: %v", err)
	}
	if err := db.Create(&Ability{Group: "default", Model: "cache-model", ChannelId: 501, Enabled: true}).Error; err != nil {
		t.Fatalf("insert ability: %v", err)
	}

	firstBuilt := make(chan struct{})
	releaseFirst := make(chan struct{})
	var once sync.Once
	channelRefreshHook = func() {
		once.Do(func() {
			close(firstBuilt)
			<-releaseFirst
		})
	}
	firstDone := make(chan error, 1)
	go func() { firstDone <- refreshChannelCache() }()
	select {
	case <-firstBuilt:
	case <-time.After(time.Second):
		t.Fatal("first channel refresh did not reach publication hook")
	}
	if err := db.Model(&Channel{}).Where("id = ?", 501).Update("name", "newer").Error; err != nil {
		t.Fatalf("update channel name: %v", err)
	}
	secondStarted := make(chan struct{})
	secondDone := make(chan error, 1)
	go func() {
		close(secondStarted)
		secondDone <- refreshChannelCache()
	}()
	select {
	case <-secondStarted:
	case <-time.After(time.Second):
		t.Fatal("second channel refresh did not start")
	}
	close(releaseFirst)
	for name, done := range map[string]<-chan error{"first": firstDone, "second": secondDone} {
		select {
		case err := <-done:
			if err != nil {
				t.Fatalf("%s refresh: %v", name, err)
			}
		case <-time.After(time.Second):
			t.Fatalf("%s channel refresh did not finish", name)
		}
	}

	channelSyncLock.RLock()
	name := channelsIDM[501].Name
	channelSyncLock.RUnlock()
	if name != "newer" {
		t.Fatalf("final channel snapshot = %q, want newer", name)
	}
}

func TestSuccessfulChannelDeleteRemainsFailClosedAfterRefreshFailure(t *testing.T) {
	preserveChannelTestState(t)
	db := openCacheTestDB(t, &Channel{}, &Ability{})
	DB = db
	common.MemoryCacheEnabled = true
	channel := &Channel{
		Id:     601,
		Name:   "delete-me",
		Status: common.ChannelStatusEnabled,
		Group:  "default",
		Models: "delete-model",
	}
	if err := db.Create(channel).Error; err != nil {
		t.Fatalf("insert channel: %v", err)
	}
	if err := db.Create(&Ability{Group: "default", Model: "delete-model", ChannelId: 601, Enabled: true}).Error; err != nil {
		t.Fatalf("insert ability: %v", err)
	}
	if err := refreshChannelCache(); err != nil {
		t.Fatalf("initial channel refresh: %v", err)
	}
	if err := channel.Delete(); err != nil {
		t.Fatalf("delete channel: %v", err)
	}
	sqlDB, err := db.DB()
	if err != nil {
		t.Fatalf("get database handle: %v", err)
	}
	if err := sqlDB.Close(); err != nil {
		t.Fatalf("close database: %v", err)
	}
	InitChannelCache()

	selected, err := GetRandomSatisfiedChannel("default", "delete-model", 0, "")
	if err != nil {
		t.Fatalf("select channel from last-known-good cache: %v", err)
	}
	if selected != nil {
		t.Fatalf("deleted channel remained routable after refresh failure: %#v", selected)
	}
}

func TestSuccessfulTagDisableRemainsFailClosedAfterRefreshFailure(t *testing.T) {
	preserveChannelTestState(t)
	db := openCacheTestDB(t, &Channel{}, &Ability{})
	DB = db
	common.MemoryCacheEnabled = true
	tag := "disable-tag"
	channel := &Channel{
		Id:     602,
		Name:   "disable-me",
		Status: common.ChannelStatusEnabled,
		Group:  "default",
		Models: "disable-model",
		Tag:    &tag,
	}
	if err := db.Create(channel).Error; err != nil {
		t.Fatalf("insert channel: %v", err)
	}
	if err := db.Create(&Ability{Group: "default", Model: "disable-model", ChannelId: 602, Enabled: true, Tag: &tag}).Error; err != nil {
		t.Fatalf("insert ability: %v", err)
	}
	if err := refreshChannelCache(); err != nil {
		t.Fatalf("initial channel refresh: %v", err)
	}
	if err := DisableChannelByTag(tag); err != nil {
		t.Fatalf("disable channels by tag: %v", err)
	}
	sqlDB, err := db.DB()
	if err != nil {
		t.Fatalf("get database handle: %v", err)
	}
	if err := sqlDB.Close(); err != nil {
		t.Fatalf("close database: %v", err)
	}
	InitChannelCache()

	selected, err := GetRandomSatisfiedChannel("default", "disable-model", 0, "")
	if err != nil {
		t.Fatalf("select channel from last-known-good cache: %v", err)
	}
	if selected != nil {
		t.Fatalf("disabled channel remained routable after refresh failure: %#v", selected)
	}
}

func TestChannelColdStartFailureIsExplicitAndRetryable(t *testing.T) {
	preserveChannelTestState(t)
	db := openCacheTestDB(t, &Channel{}, &Ability{})
	DB = db
	common.MemoryCacheEnabled = true
	sqlDB, err := db.DB()
	if err != nil {
		t.Fatalf("get database handle: %v", err)
	}
	if err := sqlDB.Close(); err != nil {
		t.Fatalf("close database: %v", err)
	}
	channelSyncLock.Lock()
	group2model2channels = nil
	channelsIDM = nil
	channel2advancedCustomConfig = make(map[int]*dto.AdvancedCustomConfig)
	channelCacheReady = false
	channelCacheLastError = nil
	channelSyncLock.Unlock()

	if err := refreshChannelCache(); err == nil {
		t.Fatal("cold-start channel refresh unexpectedly succeeded")
	}
	if _, err := GetRandomSatisfiedChannel("default", "model", 0, ""); err == nil {
		t.Fatal("cold-start cache failure was indistinguishable from an empty cache")
	}
}

func TestChannelGettersAndStatusUpdatesUseDefensiveCopyOnWrite(t *testing.T) {
	preserveChannelTestState(t)
	baseURL := "https://internal.example"
	internal := &Channel{
		Id:      801,
		Name:    "immutable",
		Status:  common.ChannelStatusEnabled,
		Group:   "default",
		Models:  "immutable-model",
		BaseURL: &baseURL,
		ChannelInfo: ChannelInfo{
			IsMultiKey:             true,
			MultiKeyStatusList:     map[int]int{1: common.ChannelStatusManuallyDisabled},
			MultiKeyDisabledReason: map[int]string{1: "sentinel"},
		},
	}
	channelSyncLock.Lock()
	channelsIDM = map[int]*Channel{801: internal}
	group2model2channels = map[string]map[string][]int{"default": {"immutable-model": {801}}}
	channel2advancedCustomConfig = make(map[int]*dto.AdvancedCustomConfig)
	channelCacheReady = true
	channelSyncLock.Unlock()
	common.MemoryCacheEnabled = true

	channel, err := CacheGetChannel(801)
	if err != nil {
		t.Fatalf("get channel: %v", err)
	}
	channel.Name = "caller-mutated"
	*channel.BaseURL = "https://caller.example"
	channel.ChannelInfo.MultiKeyStatusList[1] = common.ChannelStatusEnabled
	channel.ChannelInfo.MultiKeyDisabledReason[1] = "caller"
	info, err := CacheGetChannelInfo(801)
	if err != nil {
		t.Fatalf("get channel info: %v", err)
	}
	info.MultiKeyStatusList[1] = common.ChannelStatusEnabled
	selected, err := GetRandomSatisfiedChannel("default", "immutable-model", 0, "")
	if err != nil || selected == nil {
		t.Fatalf("select channel: channel=%#v err=%v", selected, err)
	}
	selected.Name = "selection-mutated"
	selected.ChannelInfo.MultiKeyDisabledReason[1] = "selection"

	channelSyncLock.RLock()
	if channelsIDM[801].Name != "immutable" || *channelsIDM[801].BaseURL != "https://internal.example" ||
		channelsIDM[801].ChannelInfo.MultiKeyStatusList[1] != common.ChannelStatusManuallyDisabled ||
		channelsIDM[801].ChannelInfo.MultiKeyDisabledReason[1] != "sentinel" {
		channelSyncLock.RUnlock()
		t.Fatal("channel getter exposed mutable internal cache storage")
	}
	before := channelsIDM[801]
	channelSyncLock.RUnlock()

	CacheUpdateChannelStatus(801, common.ChannelStatusManuallyDisabled)
	channelSyncLock.RLock()
	after := channelsIDM[801]
	channelSyncLock.RUnlock()
	if before == after || before.Status != common.ChannelStatusEnabled || after.Status != common.ChannelStatusManuallyDisabled {
		t.Fatal("channel status update did not publish a copy-on-write channel")
	}
}

func TestChannelSelectionUsesDistinctPriorityRetries(t *testing.T) {
	preserveChannelTestState(t)
	priorities := []int64{100, 10, 100, 50}
	channelsIDM = make(map[int]*Channel, len(priorities))
	ids := make([]int, len(priorities))
	for index := range priorities {
		id := index + 1
		ids[index] = id
		priority := priorities[index]
		weight := uint(100)
		channelsIDM[id] = &Channel{Id: id, Priority: &priority, Weight: &weight}
	}
	group2model2channels = map[string]map[string][]int{"default": {"model": ids}}
	channel2advancedCustomConfig = map[int]*dto.AdvancedCustomConfig{}
	channelCacheReady = true
	common.MemoryCacheEnabled = true

	for _, test := range []struct {
		retry    int
		priority int64
	}{{-1, 100}, {0, 100}, {1, 50}, {2, 10}, {9, 10}} {
		selected, err := GetRandomSatisfiedChannel("default", "model", test.retry, "")
		if err != nil || selected == nil || selected.GetPriority() != test.priority {
			t.Fatalf("retry=%d selected=%#v err=%v", test.retry, selected, err)
		}
	}
}

func TestOrdinaryChannelPathFilterReusesCandidateSlice(t *testing.T) {
	preserveChannelTestState(t)
	channels := []int{1, 2, 3}
	channelsIDM = map[int]*Channel{
		1: {Id: 1},
		2: {Id: 2},
		3: {Id: 3},
	}
	channel2advancedCustomConfig = map[int]*dto.AdvancedCustomConfig{}

	filtered := filterChannelsByRequestPathAndModel(channels, "/v1/responses", "model")
	if len(filtered) != len(channels) || &filtered[0] != &channels[0] {
		t.Fatalf("ordinary route filter copied its unchanged input: %v", filtered)
	}
	if allocations := testing.AllocsPerRun(1000, func() {
		_ = filterChannelsByRequestPathAndModel(channels, "/v1/responses", "model")
	}); allocations != 0 {
		t.Fatalf("ordinary route filter allocations=%f, want 0", allocations)
	}
}

func setupMutationPathTest(t *testing.T, target *Channel, unrelatedID int) *gorm.DB {
	t.Helper()
	db := openCacheTestDB(t, &Channel{}, &Ability{})
	DB = db
	common.MemoryCacheEnabled = true
	unrelated := &Channel{
		Id:     unrelatedID,
		Name:   "unrelated",
		Status: common.ChannelStatusEnabled,
		Group:  "default",
		Models: "unrelated-model",
	}
	if err := db.Create(target).Error; err != nil {
		t.Fatalf("insert target channel: %v", err)
	}
	if err := db.Create(unrelated).Error; err != nil {
		t.Fatalf("insert unrelated channel: %v", err)
	}
	for _, ability := range []Ability{
		{Group: "default", Model: target.Models, ChannelId: target.Id, Enabled: target.Status == common.ChannelStatusEnabled, Tag: target.Tag},
		{Group: "default", Model: unrelated.Models, ChannelId: unrelated.Id, Enabled: true},
	} {
		if err := db.Create(&ability).Error; err != nil {
			t.Fatalf("insert ability: %v", err)
		}
	}
	if err := refreshChannelCache(); err != nil {
		t.Fatalf("refresh channel cache: %v", err)
	}
	return db
}

func assertUnrelatedChannelRoutable(t *testing.T, unrelatedID int) {
	t.Helper()
	channel, err := GetRandomSatisfiedChannel("default", "unrelated-model", 0, "")
	if err != nil {
		t.Fatalf("select unrelated channel: %v", err)
	}
	if channel == nil || channel.Id != unrelatedID {
		t.Fatalf("unrelated enabled channel was removed: %#v", channel)
	}
}

func assertChannelRemovedFromCache(t *testing.T, channelID int) {
	t.Helper()
	if channel, err := CacheGetChannel(channelID); err == nil || channel != nil {
		t.Fatalf("affected channel %d remained in cache: channel=%#v err=%v", channelID, channel, err)
	}
}

func TestBatchMutationPathsRemoveOnlyActuallyAffectedChannels(t *testing.T) {
	t.Run("disable by tag", func(t *testing.T) {
		preserveChannelTestState(t)
		tag := "target-tag"
		setupMutationPathTest(t, &Channel{Id: 901, Name: "target", Status: common.ChannelStatusEnabled, Group: "default", Models: "target-model", Tag: &tag}, 902)
		if err := DisableChannelByTag(tag); err != nil {
			t.Fatalf("disable by tag: %v", err)
		}
		assertChannelRemovedFromCache(t, 901)
		assertUnrelatedChannelRoutable(t, 902)
	})

	t.Run("batch delete", func(t *testing.T) {
		preserveChannelTestState(t)
		setupMutationPathTest(t, &Channel{Id: 911, Name: "target", Status: common.ChannelStatusEnabled, Group: "default", Models: "target-model"}, 912)
		count, err := BatchDeleteChannels([]int{911, 999999})
		if err != nil || count != 1 {
			t.Fatalf("batch delete count=%d err=%v, want 1", count, err)
		}
		assertChannelRemovedFromCache(t, 911)
		assertUnrelatedChannelRoutable(t, 912)
	})

	t.Run("delete by status", func(t *testing.T) {
		preserveChannelTestState(t)
		setupMutationPathTest(t, &Channel{Id: 921, Name: "target", Status: 9, Group: "default", Models: "target-model"}, 922)
		count, err := DeleteChannelByStatus(9)
		if err != nil || count != 1 {
			t.Fatalf("delete by status count=%d err=%v, want 1", count, err)
		}
		assertChannelRemovedFromCache(t, 921)
		assertUnrelatedChannelRoutable(t, 922)
	})

	t.Run("delete disabled", func(t *testing.T) {
		preserveChannelTestState(t)
		setupMutationPathTest(t, &Channel{Id: 931, Name: "target", Status: common.ChannelStatusManuallyDisabled, Group: "default", Models: "target-model"}, 932)
		count, err := DeleteDisabledChannel()
		if err != nil || count != 1 {
			t.Fatalf("delete disabled count=%d err=%v, want 1", count, err)
		}
		assertChannelRemovedFromCache(t, 931)
		assertUnrelatedChannelRoutable(t, 932)
	})
}

func TestCacheReadinessRecoversAfterColdWarmFailure(t *testing.T) {
	preservePricingTestState(t)
	preserveChannelTestState(t)
	previousEnforced := cacheReadinessEnforced.Load()
	t.Cleanup(func() { cacheReadinessEnforced.Store(previousEnforced) })
	common.MemoryCacheEnabled = true
	pricingCache.Store(nil)
	channelSyncLock.Lock()
	channelCacheReady = false
	channelCacheLastError = nil
	channelSyncLock.Unlock()

	closedDB := openCacheTestDB(t, &Channel{}, &Ability{}, &Model{}, &Vendor{})
	sqlDB, err := closedDB.DB()
	if err != nil {
		t.Fatalf("get closed database handle: %v", err)
	}
	if err := sqlDB.Close(); err != nil {
		t.Fatalf("close database: %v", err)
	}
	DB = closedDB
	if err := WarmCaches(); err == nil {
		t.Fatal("cold warm unexpectedly succeeded against closed database")
	}
	if CachesReady() || CacheReadinessError() == nil {
		t.Fatal("failed cold warm reported ready")
	}

	DB = openCacheTestDB(t, &Channel{}, &Ability{}, &Model{}, &Vendor{})
	if err := WarmCaches(); err != nil {
		t.Fatalf("retry warm caches: %v", err)
	}
	if !CachesReady() || CacheReadinessError() != nil {
		t.Fatal("successful retry did not restore readiness")
	}
}

func TestChannelCopiesPollingStatusAndRefreshAreRaceSafe(t *testing.T) {
	preserveChannelTestState(t)
	db := openCacheTestDB(t, &Channel{}, &Ability{})
	DB = db
	common.MemoryCacheEnabled = true
	channel := &Channel{
		Id:     1001,
		Name:   "race-channel",
		Key:    "key-a\nkey-b",
		Status: common.ChannelStatusEnabled,
		Group:  "default",
		Models: "race-model",
		ChannelInfo: ChannelInfo{
			IsMultiKey:   true,
			MultiKeySize: 2,
			MultiKeyMode: constant.MultiKeyModePolling,
		},
	}
	if err := db.Create(channel).Error; err != nil {
		t.Fatalf("insert channel: %v", err)
	}
	if err := db.Create(&Ability{Group: "default", Model: "race-model", ChannelId: 1001, Enabled: true}).Error; err != nil {
		t.Fatalf("insert ability: %v", err)
	}
	if err := refreshChannelCache(); err != nil {
		t.Fatalf("initial refresh: %v", err)
	}

	var workers sync.WaitGroup
	errCh := make(chan error, 16)
	for i := 0; i < 4; i++ {
		workers.Add(1)
		go func() {
			defer workers.Done()
			for j := 0; j < 100; j++ {
				channel, err := CacheGetChannel(1001)
				if err != nil {
					errCh <- err
					return
				}
				channel.Name = "caller-local"
				if _, _, apiErr := channel.GetNextEnabledKey(); apiErr != nil {
					errCh <- apiErr
					return
				}
			}
		}()
	}
	workers.Add(1)
	go func() {
		defer workers.Done()
		for i := 0; i < 30; i++ {
			status := common.ChannelStatusEnabled
			if i%2 == 0 {
				status = common.ChannelStatusManuallyDisabled
			}
			if !UpdateChannelStatus(1001, "key-a", status, "race test") {
				errCh <- errors.New("multi-key status update failed")
				return
			}
		}
	}()
	workers.Add(1)
	go func() {
		defer workers.Done()
		for i := 0; i < 25; i++ {
			if err := refreshChannelCache(); err != nil {
				errCh <- err
				return
			}
		}
	}()
	done := make(chan struct{})
	go func() {
		workers.Wait()
		close(done)
	}()
	select {
	case <-done:
	case <-time.After(5 * time.Second):
		t.Fatal("channel race exercise timed out")
	}
	select {
	case err := <-errCh:
		t.Fatalf("channel race exercise: %v", err)
	default:
	}
}
