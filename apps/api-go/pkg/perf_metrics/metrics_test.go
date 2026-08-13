// Copyright (c) 2025-2026 QuantumNous. All rights reserved.

package perfmetrics

import (
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
