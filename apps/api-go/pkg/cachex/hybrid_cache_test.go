package cachex

import (
	"context"
	"strconv"
	"testing"

	"github.com/alicebob/miniredis/v2"
	"github.com/go-redis/redis/v8"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func newRedisIntCacheForTest(t *testing.T, namespace string) (*HybridCache[int], *miniredis.Miniredis) {
	t.Helper()
	server := miniredis.RunT(t)
	client := redis.NewClient(&redis.Options{Addr: server.Addr()})
	t.Cleanup(func() { _ = client.Close() })
	cache := NewHybridCache[int](HybridCacheConfig[int]{
		Namespace:    Namespace(namespace),
		Redis:        client,
		RedisCodec:   IntCodec{},
		RedisEnabled: func() bool { return true },
	})
	return cache, server
}

func TestHybridCacheForEachKeyDoesNotChangeKeySemantics(t *testing.T) {
	cache, server := newRedisIntCacheForTest(t, "cachex:test:foreach")
	for i := 0; i < 2505; i++ {
		require.NoError(t, cache.SetWithTTL("key:"+strconv.Itoa(i), i, 0))
	}
	// Add a key outside the namespace to ensure SCAN's match pattern remains
	// isolated from unrelated Redis data.
	server.Set("other:key", "1")

	seen := make(map[string]struct{})
	err := cache.ForEachKey(func(key string) error {
		seen[key] = struct{}{}
		return nil
	})
	require.NoError(t, err)
	assert.Len(t, seen, 2505)
	assert.NotContains(t, seen, "other:key")

	keys, err := cache.Keys()
	require.NoError(t, err)
	assert.Len(t, keys, 2505)
}

func TestHybridCacheRedisDeletesInScanBatches(t *testing.T) {
	cache, server := newRedisIntCacheForTest(t, "cachex:test:delete")
	for i := 0; i < 2505; i++ {
		require.NoError(t, cache.SetWithTTL("rule:value:"+strconv.Itoa(i), i, 0))
	}
	for i := 0; i < 5; i++ {
		require.NoError(t, cache.SetWithTTL("keep:value:"+strconv.Itoa(i), i, 0))
	}

	deleted, err := cache.DeleteByPrefix("rule")
	require.NoError(t, err)
	assert.Equal(t, 2505, deleted)

	deleted, err = cache.DeleteAll()
	require.NoError(t, err)
	assert.Equal(t, 5, deleted)
	assert.Empty(t, server.Keys())

	// Purge remains safe when the namespace is already empty.
	assert.NoError(t, cache.Purge())
	assert.Empty(t, server.Keys())
	assert.NoError(t, cache.redis.Ping(context.Background()).Err())
}
