package cachex

import (
	"fmt"
	"testing"
	"time"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestByteCacheHonorsCountAndByteBudgets(t *testing.T) {
	cache := NewByteCache[string](3, 7, func(key, value string) int64 {
		return int64(len(key) + len(value))
	})
	cache.SetWithTTL("a", "123", time.Minute)
	cache.SetWithTTL("b", "456", time.Minute)
	assert.LessOrEqual(t, cache.Bytes(), int64(7))
	_, firstFound, err := cache.Get("a")
	require.NoError(t, err)
	_, secondFound, err := cache.Get("b")
	require.NoError(t, err)
	assert.NotEqual(t, firstFound, secondFound, "the byte budget must evict one entry")
	cache.SetWithTTL("oversized", "value", time.Minute)
	_, found, err := cache.Get("oversized")
	require.NoError(t, err)
	assert.False(t, found)
}

func TestByteCacheComputeIsAtomicAndBounded(t *testing.T) {
	cache := NewByteCache[int](32, 1<<10, func(key string, _ int) int64 { return int64(len(key) + 8) })
	for i := 0; i < 10_000; i++ {
		key := fmt.Sprintf("key-%d", i)
		value, stored := cache.Compute(key, time.Minute, func(current int, _ bool) (int, bool) {
			return current + 1, true
		})
		require.True(t, stored)
		require.Equal(t, 1, value)
	}
	assert.LessOrEqual(t, cache.Len(), 32)
	assert.LessOrEqual(t, cache.Bytes(), int64(1<<10))
}

func TestByteCacheExpiresWithoutJanitor(t *testing.T) {
	cache := NewByteCache[string](2, 64, func(key, value string) int64 { return int64(len(key) + len(value)) })
	cache.SetWithTTL("key", "value", time.Nanosecond)
	time.Sleep(time.Millisecond)
	_, found, err := cache.Get("key")
	require.NoError(t, err)
	assert.False(t, found)
	assert.Zero(t, cache.Bytes())
}

func TestByteCacheMapCompatibilityRemainsBounded(t *testing.T) {
	cache := NewByteCache[int](1, 64, func(key string, _ int) int64 { return int64(len(key) + 8) })
	actual, loaded := cache.LoadOrStore("first", 1)
	assert.False(t, loaded)
	assert.Equal(t, 1, actual)
	actual, loaded = cache.LoadOrStore("first", 2)
	assert.True(t, loaded)
	assert.Equal(t, 1, actual)
	cache.Store("second", 2)
	_, found := cache.Load("first")
	assert.False(t, found)
	cache.Delete("second")
	assert.Empty(t, cache.Keys())
}
