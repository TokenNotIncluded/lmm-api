package service

import (
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
