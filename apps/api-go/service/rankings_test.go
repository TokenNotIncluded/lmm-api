package service

import (
	"fmt"
	"testing"
	"time"

	"github.com/LIghtJUNction/api.lmm.best/model"
	"github.com/stretchr/testify/require"
)

func TestRankingBucketSummaryPreservesHistoryProjection(t *testing.T) {
	config := rankingPeriodConfig{bucketSize: 3600, labelLayout: "15:04"}
	meta := map[string]rankingModelMeta{
		"m0":  {vendor: "v0"},
		"m1":  {vendor: "v1"},
		"m2":  {vendor: "v2"},
		"m3":  {vendor: "v3"},
		"m4":  {vendor: "v4"},
		"m5":  {vendor: "v5"},
		"m6":  {vendor: "v5"},
		"m7":  {vendor: "v5"},
		"m8":  {vendor: "v5"},
		"m9":  {vendor: "v5"},
		"m10": {vendor: "v5"},
	}
	totals := []model.RankingQuotaTotal{
		{ModelName: "m0", TotalTokens: 100},
		{ModelName: "m1", TotalTokens: 90},
		{ModelName: "m2", TotalTokens: 80},
		{ModelName: "m3", TotalTokens: 70},
		{ModelName: "m4", TotalTokens: 60},
		{ModelName: "m5", TotalTokens: 50},
		{ModelName: "m6", TotalTokens: 40},
		{ModelName: "m7", TotalTokens: 30},
		{ModelName: "m8", TotalTokens: 20},
		{ModelName: "m9", TotalTokens: 10},
		{ModelName: "m10", TotalTokens: 5},
	}
	vendors := []RankedVendor{
		{Vendor: "v5", TotalTokens: 155, Share: rankingShare(155, 555)},
		{Vendor: "v0", TotalTokens: 100, Share: rankingShare(100, 555)},
		{Vendor: "v1", TotalTokens: 90, Share: rankingShare(90, 555)},
		{Vendor: "v2", TotalTokens: 80, Share: rankingShare(80, 555)},
		{Vendor: "v3", TotalTokens: 70, Share: rankingShare(70, 555)},
		{Vendor: "v4", TotalTokens: 60, Share: rankingShare(60, 555)},
	}
	buckets := make([]model.RankingQuotaBucket, 0, len(totals)*2)
	for idx, item := range totals {
		buckets = append(buckets,
			model.RankingQuotaBucket{ModelName: item.ModelName, Bucket: 0, Tokens: item.TotalTokens},
			model.RankingQuotaBucket{ModelName: item.ModelName, Bucket: int64(time.Hour.Seconds()), Tokens: int64(idx + 1)},
		)
	}

	summary := newRankingBucketSummary(totals, vendors)
	for _, item := range buckets {
		summary.add(item, meta)
	}

	legacyModels := buildModelHistory(buckets, totals, meta, config)
	streamedModels := buildModelHistoryFromSummary(summary, totals, meta, config)
	require.Equal(t, legacyModels, streamedModels)

	legacyVendors := buildVendorShareHistory(buckets, vendors, 555, meta, config)
	streamedVendors := buildVendorShareHistoryFromSummary(summary, vendors, 555, config)
	require.Equal(t, legacyVendors, streamedVendors)
}

func TestRankedModelLimitAndMoverSelectionPreserveResults(t *testing.T) {
	totals := make([]model.RankingQuotaTotal, 30)
	previousRanks := make(map[string]int, len(totals))
	previousTokens := make(map[string]int64, len(totals))
	for idx := range totals {
		name := fmt.Sprintf("m%02d", idx)
		totals[idx] = model.RankingQuotaTotal{ModelName: name, TotalTokens: int64(1000 - idx)}
		previousRanks[name] = idx + 1
		previousTokens[name] = int64(1000 - idx)
	}
	// Seven models share the same positive delta. Their different growth
	// values exercise the secondary ordering while the result remains capped.
	for idx := 0; idx < 7; idx++ {
		name := fmt.Sprintf("m%02d", idx)
		previousRanks[name] = idx + 10
		previousTokens[name] = int64((idx + 1) * 100)
	}
	// Nine models share the same negative delta and exercise the dropper path.
	for idx := 7; idx < 16; idx++ {
		name := fmt.Sprintf("m%02d", idx)
		previousRanks[name] = idx - 6
		previousTokens[name] = int64((idx - 6) * 100)
	}

	meta := make(map[string]rankingModelMeta, len(totals))
	for _, item := range totals {
		meta[item.ModelName] = rankingModelMeta{vendor: "test"}
	}
	totalTokens := sumRankingTokens(totals)
	legacyModels := buildRankedModels(totals, totalTokens, previousRanks, previousTokens, meta, true)
	legacyMovers, legacyDroppers := buildRankingMovers(legacyModels)
	limitedModels := buildRankedModelsLimit(totals, totalTokens, previousRanks, previousTokens, meta, true, rankingLeaderboardLimit)
	selectedMovers, selectedDroppers := buildRankingMoversFromTotals(totals, previousRanks, previousTokens, meta, true)

	require.Len(t, limitedModels, rankingLeaderboardLimit)
	require.Equal(t, legacyModels[:rankingLeaderboardLimit], limitedModels)
	require.Equal(t, legacyMovers, selectedMovers)
	require.Equal(t, legacyDroppers, selectedDroppers)
}
