package service

import (
	"fmt"
	"math"
	"strings"
	"testing"

	"github.com/QuantumNous/new-api/common"
	"github.com/QuantumNous/new-api/model"
	"github.com/QuantumNous/new-api/pkg/dynamic_pricing"
	"github.com/QuantumNous/new-api/setting/config"
	"github.com/QuantumNous/new-api/setting/dynamic_pricing_setting"

	"github.com/glebarez/sqlite"
	"github.com/stretchr/testify/require"
	"gorm.io/gorm"
)

// setupDynamicPricingTestDB swaps in a dedicated in-memory LOG_DB and a
// deterministic dynamic pricing config, restoring both on cleanup.
func setupDynamicPricingTestDB(t *testing.T) (*gorm.DB, int64) {
	t.Helper()

	previousLogDB := model.LOG_DB
	previousRedisEnabled := common.RedisEnabled
	dsn := fmt.Sprintf("file:%s?mode=memory&cache=shared", strings.ReplaceAll(t.Name(), "/", "_"))
	db, err := gorm.Open(sqlite.Open(dsn), &gorm.Config{})
	require.NoError(t, err)
	model.LOG_DB = db
	require.NoError(t, db.AutoMigrate(&model.Log{}))
	common.RedisEnabled = false

	cfg, ok := config.GlobalConfig.Get("dynamic_pricing_setting").(*dynamic_pricing_setting.DynamicPricingSetting)
	require.True(t, ok, "dynamic_pricing_setting not registered")
	oldCfg := dynamic_pricing_setting.GetSetting()
	cfg.Enabled = true
	cfg.WindowMinutes = 5
	cfg.TargetTPM = 100000
	cfg.TargetRPM = 60
	cfg.TargetCostRate = 1.0
	cfg.AlphaLoad = 0.3
	cfg.AlphaUp = 0.30
	cfg.AlphaDown = 0.05
	cfg.CostFloorFactor = 1.2
	cfg.MaxFactor = 3.0
	cfg.LoadDeadzone = 0.4
	cfg.HeatGamma = 2.0
	cfg.MaxStepUp = 0.10
	cfg.MaxStepDown = 0.03
	cfg.FailoverProbability = 0.15
	cfg.ChannelCosts = map[string]float64{"1": 2.0, "2": 1.0, "4": 1.8}

	now := common.GetTimestamp()
	t.Cleanup(func() {
		*cfg = oldCfg
		model.LOG_DB = previousLogDB
		common.RedisEnabled = previousRedisEnabled
		sqlDB, dbErr := db.DB()
		if dbErr == nil {
			_ = sqlDB.Close()
		}
	})
	return db, now
}

func seedDynamicPricingLog(t *testing.T, db *gorm.DB, modelName string, channelId int, logType int, promptTokens, completionTokens int, createdAt int64) {
	t.Helper()
	log := model.Log{
		UserId:           1,
		Type:             logType,
		ModelName:        modelName,
		ChannelId:        channelId,
		PromptTokens:     promptTokens,
		CompletionTokens: completionTokens,
		CreatedAt:        createdAt,
	}
	require.NoError(t, db.Create(&log).Error)
}

func approxEq(a, b, eps float64) bool { return math.Abs(a-b) <= eps }

func TestRunDynamicPricingTick(t *testing.T) {
	db, now := setupDynamicPricingTestDB(t)

	// m1: one configured channel (cost 2.0 USD/1M), 5 requests, 1,000,000 tokens.
	for i := 0; i < 5; i++ {
		seedDynamicPricingLog(t, db, "m1", 1, model.LogTypeConsume, 120000, 80000, now-60-int64(i)*10)
	}
	// m2: channels 2 (1.0) and 4 (1.8) configured, channel 3 without cost.
	for i := 0; i < 2; i++ {
		seedDynamicPricingLog(t, db, "m2", 2, model.LogTypeConsume, 100000, 0, now-120-int64(i)*10)
		seedDynamicPricingLog(t, db, "m2", 4, model.LogTypeConsume, 150000, 0, now-130-int64(i)*10)
		seedDynamicPricingLog(t, db, "m2", 3, model.LogTypeConsume, 100000, 0, now-140-int64(i)*10)
	}
	// m3: only a channel without configured cost -> no cost signal, base price.
	for i := 0; i < 3; i++ {
		seedDynamicPricingLog(t, db, "m3", 5, model.LogTypeConsume, 100000, 0, now-90-int64(i)*10)
	}
	// Outside the window: must be ignored.
	seedDynamicPricingLog(t, db, "m-old", 1, model.LogTypeConsume, 1000000, 0, now-600)
	// Wrong log type (manage): must be ignored.
	seedDynamicPricingLog(t, db, "m-manage", 1, model.LogTypeManage, 1000000, 0, now-60)

	before := common.GetTimestamp()
	runDynamicPricingTick()
	after := common.GetTimestamp()

	// m1: tokens=1e6, cost=1e6/1e6*2.0=2.0 -> unit cost 2.0; raw load=1e6/5/1e5=2.0.
	st, ok := dynamic_pricing.GetState("m1")
	require.True(t, ok, "m1 state missing")
	require.True(t, approxEq(st.LoadEMA, 2.0, 1e-9), "m1 LoadEMA = %v, want 2.0", st.LoadEMA)
	require.True(t, approxEq(st.CostEMA, 2.0, 1e-9), "m1 CostEMA = %v, want 2.0", st.CostEMA)
	// Cold start seeds Factor=1.0, so the first tick is step-clamped to
	// 1.0*(1+MaxStepUp)=1.1 instead of jumping straight to maxFactor.
	require.True(t, approxEq(st.Factor, 1.1, 1e-9), "m1 Factor = %v, want 1.1 (first-tick step-up clamp)", st.Factor)
	require.True(t, st.UpdatedAt >= before && st.UpdatedAt <= after, "m1 UpdatedAt = %d outside [%d, %d]", st.UpdatedAt, before, after)

	// m2: total tokens=7e5, but only 5e5 priced tokens (ch3 is unknown),
	// cost=0.2+0.54=0.74, unit cost=0.74e6/5e5; raw load uses only the
	// priced denominator and is therefore 1.0.
	st, ok = dynamic_pricing.GetState("m2")
	require.True(t, ok, "m2 state missing")
	require.True(t, approxEq(st.LoadEMA, 1.0, 1e-9), "m2 LoadEMA = %v, want 1.0", st.LoadEMA)
	require.True(t, approxEq(st.CostEMA, 0.74e6/5e5, 1e-6), "m2 CostEMA = %v, want %v", st.CostEMA, 0.74e6/5e5)
	require.True(t, approxEq(st.Factor, 1.1, 1e-9), "m2 Factor = %v, want 1.1 (first-tick step-up clamp)", st.Factor)

	// m3: no configured cost -> factor stays at base price.
	if got := dynamic_pricing.GetMultiplier("m3"); got != 1.0 {
		t.Fatalf("m3 multiplier = %v, want 1.0", got)
	}

	// Models outside the window / wrong type must have no state.
	if _, ok := dynamic_pricing.GetState("m-old"); ok {
		t.Fatal("m-old must not be ticked (outside window)")
	}
	if _, ok := dynamic_pricing.GetState("m-manage"); ok {
		t.Fatal("m-manage must not be ticked (non-consume log type)")
	}
}

func TestAggregateDynamicPricingWindow(t *testing.T) {
	setupDynamicPricingTestDB(t) // sets ChannelCosts: 1=2.0, 2=1.0, 4=1.8

	rows := []dynamicPricingWindowRow{
		{ModelName: "m1", ChannelId: 1, Tokens: 1000000, Requests: 5},
		{ModelName: "m1", ChannelId: 1, Tokens: 500000, Requests: 2}, // same channel, same model: merged
		{ModelName: "m2", ChannelId: 2, Tokens: 200000, Requests: 2},
		{ModelName: "m2", ChannelId: 4, Tokens: 300000, Requests: 2},
		{ModelName: "m2", ChannelId: 3, Tokens: 200000, Requests: 2}, // no configured cost
		{ModelName: "m3", ChannelId: 5, Tokens: 300000, Requests: 3}, // no configured cost at all
	}
	perModel := aggregateDynamicPricingWindow(rows)

	m1 := perModel["m1"]
	require.NotNil(t, m1)
	require.True(t, approxEq(m1.tokens, 1500000, 1e-9), "m1 tokens = %v", m1.tokens)
	require.True(t, approxEq(m1.requests, 7, 1e-9), "m1 requests = %v", m1.requests)
	require.True(t, approxEq(m1.costUSD, 1.5e6/1e6*2.0, 1e-9), "m1 costUSD = %v", m1.costUSD)
	require.True(t, approxEq(m1.cheap, 2.0, 1e-9), "m1 cheap = %v, want 2.0", m1.cheap)
	require.True(t, approxEq(m1.backup, 0, 1e-9), "m1 backup = %v, want 0 (single channel)", m1.backup)

	m2 := perModel["m2"]
	require.NotNil(t, m2)
	require.True(t, approxEq(m2.tokens, 700000, 1e-9), "m2 tokens = %v", m2.tokens)
	require.True(t, approxEq(m2.pricedTokens, 500000, 1e-9), "m2 pricedTokens = %v", m2.pricedTokens)
	require.True(t, approxEq(m2.unpricedTokens, 200000, 1e-9), "m2 unpricedTokens = %v", m2.unpricedTokens)
	require.True(t, approxEq(m2.costUSD, 0.2+0.54, 1e-9), "m2 costUSD = %v, want 0.74", m2.costUSD)
	require.True(t, approxEq(m2.cheap, 1.0, 1e-9), "m2 cheap = %v, want 1.0", m2.cheap)
	require.True(t, approxEq(m2.backup, 1.8, 1e-9), "m2 backup = %v, want 1.8", m2.backup)

	m3 := perModel["m3"]
	require.NotNil(t, m3)
	require.True(t, approxEq(m3.costUSD, 0, 1e-9), "m3 costUSD = %v, want 0", m3.costUSD)
	require.True(t, approxEq(m3.cheap, 0, 1e-9), "m3 cheap = %v, want 0", m3.cheap)
	require.True(t, approxEq(m3.backup, 0, 1e-9), "m3 backup = %v, want 0", m3.backup)
}

func TestRunDynamicPricingTickDisabled(t *testing.T) {
	db, now := setupDynamicPricingTestDB(t)
	seedDynamicPricingLog(t, db, "m-disabled", 1, model.LogTypeConsume, 1000000, 0, now-60)

	cfg := config.GlobalConfig.Get("dynamic_pricing_setting").(*dynamic_pricing_setting.DynamicPricingSetting)
	cfg.Enabled = false

	runDynamicPricingTick()

	if _, ok := dynamic_pricing.GetState("m-disabled"); ok {
		t.Fatal("tick must be a no-op while the feature is disabled")
	}
}

func TestRunDynamicPricingTickDecaysModelsWithNoTraffic(t *testing.T) {
	setupDynamicPricingTestDB(t)
	dynamic_pricing.SetState("m-stale", &dynamic_pricing.ModelState{
		LoadEMA: 1.5,
		CostEMA: 2.0,
		Factor:  2.5,
	})

	runDynamicPricingTick()

	state, ok := dynamic_pricing.GetState("m-stale")
	require.True(t, ok, "stale model state missing")
	require.Less(t, state.Factor, 2.5, "zero-traffic tick must decay the factor")
	require.Greater(t, state.Factor, 1.0, "zero-traffic decay should be gradual")
	require.Less(t, state.LoadEMA, 1.5, "zero-traffic tick must decay load EMA")
	require.Less(t, state.CostEMA, 2.0, "zero-traffic tick must decay cost EMA")
}
