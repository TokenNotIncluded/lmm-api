// Copyright (c) 2025-2026 QuantumNous. All rights reserved.

package perfmetrics

import (
	"errors"
	"sync"
	"testing"
)

func resetHotBucketsForTest() {
	hotBuckets.Range(func(key, _ any) bool {
		hotBuckets.Delete(key)
		return true
	})
	hotBucketCount.Store(0)
	hotBucketDropped.Store(0)
}

func TestHotBucketBudgetIsBoundedAndReleasesDeletedSlots(t *testing.T) {
	resetHotBucketsForTest()
	t.Cleanup(resetHotBucketsForTest)
	first := bucketKey{model: "first", group: "default", bucketTs: 1}
	second := bucketKey{model: "second", group: "default", bucketTs: 1}
	third := bucketKey{model: "third", group: "default", bucketTs: 1}

	if _, ok := loadHotBucket(first, 2); !ok {
		t.Fatal("first bucket was rejected")
	}
	if _, ok := loadHotBucket(second, 2); !ok {
		t.Fatal("second bucket was rejected")
	}
	if _, ok := loadHotBucket(third, 2); ok {
		t.Fatal("bucket budget accepted excess cardinality")
	}
	if count := hotBucketCount.Load(); count != 2 {
		t.Fatalf("bucket count=%d, want 2", count)
	}

	deleteHotBucket(first)
	if _, ok := loadHotBucket(third, 2); !ok {
		t.Fatal("released bucket slot was not reusable")
	}
	if count := hotBucketCount.Load(); count != 2 {
		t.Fatalf("bucket count after reuse=%d, want 2", count)
	}
}

func TestHotBucketHitDoesNotAllocate(t *testing.T) {
	resetHotBucketsForTest()
	t.Cleanup(resetHotBucketsForTest)
	key := bucketKey{model: "stable", group: "default", bucketTs: 1}
	if _, ok := loadHotBucket(key, 2); !ok {
		t.Fatal("warm bucket was rejected")
	}
	if allocations := testing.AllocsPerRun(1000, func() {
		_, _ = loadHotBucket(key, 2)
	}); allocations != 0 {
		t.Fatalf("hot bucket hit allocations=%f, want 0", allocations)
	}
}

func TestHotBucketConcurrentLoadCountsOneEntry(t *testing.T) {
	resetHotBucketsForTest()
	t.Cleanup(resetHotBucketsForTest)
	key := bucketKey{model: "shared", group: "default", bucketTs: 1}

	const workers = 64
	start := make(chan struct{})
	buckets := make(chan *atomicBucket, workers)
	var group sync.WaitGroup
	group.Add(workers)
	for range workers {
		go func() {
			defer group.Done()
			<-start
			bucket, ok := loadHotBucket(key, workers)
			if !ok {
				t.Error("shared bucket was rejected")
				return
			}
			buckets <- bucket
		}()
	}
	close(start)
	group.Wait()
	close(buckets)

	var first *atomicBucket
	for bucket := range buckets {
		if first == nil {
			first = bucket
			continue
		}
		if bucket != first {
			t.Fatal("same key returned multiple buckets")
		}
	}
	if count := hotBucketCount.Load(); count != 1 {
		t.Fatalf("bucket count=%d, want 1", count)
	}
}

func TestMergeModelBucketRetainsOnlyRecentSuccessRateBuckets(t *testing.T) {
	modelBuckets := map[string]map[int64]counters{}
	for ts := int64(1); ts <= 4; ts++ {
		mergeModelBucket(modelBuckets, "model", ts, counters{
			requestCount: 1,
			successCount: ts % 2,
		})
	}

	buckets := modelBuckets["model"]
	if len(buckets) != recentSuccessRateBucketLimit {
		t.Fatalf("bucket count=%d, want %d", len(buckets), recentSuccessRateBucketLimit)
	}
	if _, ok := buckets[1]; ok {
		t.Fatal("oldest bucket was retained")
	}
	for ts := int64(2); ts <= 4; ts++ {
		if _, ok := buckets[ts]; !ok {
			t.Fatalf("recent bucket %d was dropped", ts)
		}
	}
	rates := recentSuccessRates(buckets, recentSuccessRateBucketLimit)
	if len(rates) != recentSuccessRateBucketLimit || rates[0] != 0 || rates[1] != 100 || rates[2] != 0 {
		t.Fatalf("recent success rates=%v, want [0 100 0]", rates)
	}
}

func TestMergeModelSummaryRejectsExcessModelsBeforeAllocation(t *testing.T) {
	totals := map[string]counters{}
	modelBuckets := map[string]map[int64]counters{}
	value := counters{requestCount: 1, successCount: 1}

	for _, name := range []string{"first", "second"} {
		if err := mergeModelSummary(totals, modelBuckets, name, 1, value, 2); err != nil {
			t.Fatalf("merge %q: %v", name, err)
		}
	}
	if err := mergeModelSummary(totals, modelBuckets, "third", 1, value, 2); !errors.Is(err, ErrPerformanceSummaryTooManyModels) {
		t.Fatalf("third model error=%v, want model safety limit", err)
	}
	if len(totals) != 2 || len(modelBuckets) != 2 {
		t.Fatalf("maps grew past limit: totals=%d buckets=%d", len(totals), len(modelBuckets))
	}
}
