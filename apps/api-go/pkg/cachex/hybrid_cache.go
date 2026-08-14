package cachex

import (
	"context"
	"errors"
	"strings"
	"sync"
	"time"

	"github.com/go-redis/redis/v8"
	"github.com/samber/hot"
)

const (
	defaultRedisOpTimeout   = 2 * time.Second
	defaultRedisScanTimeout = 30 * time.Second
	defaultRedisDelTimeout  = 10 * time.Second
	redisScanBatchSize      = 1000
)

type HybridCacheConfig[V any] struct {
	Namespace Namespace

	// Redis is used when RedisEnabled returns true (or RedisEnabled is nil) and Redis is not nil.
	Redis        *redis.Client
	RedisCodec   ValueCodec[V]
	RedisEnabled func() bool

	// Memory builds a hot cache used when Redis is disabled. Keys stored in memory are fully namespaced.
	Memory      func() *hot.HotCache[string, V]
	MemoryStore func() MemoryCache[V]
}

// HybridCache is a small helper that uses Redis when enabled, otherwise falls back to in-memory hot cache.
type HybridCache[V any] struct {
	ns Namespace

	redis        *redis.Client
	redisCodec   ValueCodec[V]
	redisEnabled func() bool

	memOnce sync.Once
	memInit func() MemoryCache[V]
	mem     MemoryCache[V]
}

func NewHybridCache[V any](cfg HybridCacheConfig[V]) *HybridCache[V] {
	var memoryFactory func() MemoryCache[V]
	if cfg.MemoryStore != nil {
		memoryFactory = cfg.MemoryStore
	} else if cfg.Memory != nil {
		memoryFactory = func() MemoryCache[V] { return cfg.Memory() }
	}
	return &HybridCache[V]{
		ns:           cfg.Namespace,
		redis:        cfg.Redis,
		redisCodec:   cfg.RedisCodec,
		redisEnabled: cfg.RedisEnabled,
		memInit:      memoryFactory,
	}
}

func (c *HybridCache[V]) FullKey(key string) string {
	return c.ns.FullKey(key)
}

func (c *HybridCache[V]) redisOn() bool {
	if c.redis == nil || c.redisCodec == nil {
		return false
	}
	if c.redisEnabled == nil {
		return true
	}
	return c.redisEnabled()
}

func (c *HybridCache[V]) memCache() MemoryCache[V] {
	c.memOnce.Do(func() {
		if c.memInit == nil {
			c.mem = hot.NewHotCache[string, V](hot.LRU, 1).Build()
			return
		}
		c.mem = c.memInit()
	})
	return c.mem
}

func (c *HybridCache[V]) Get(key string) (value V, found bool, err error) {
	full := c.ns.FullKey(key)
	if full == "" {
		var zero V
		return zero, false, nil
	}

	if c.redisOn() {
		ctx, cancel := context.WithTimeout(context.Background(), defaultRedisOpTimeout)
		defer cancel()

		raw, e := c.redis.Get(ctx, full).Result()
		if e == nil {
			v, decErr := c.redisCodec.Decode(raw)
			if decErr != nil {
				var zero V
				return zero, false, decErr
			}
			return v, true, nil
		}
		if errors.Is(e, redis.Nil) {
			var zero V
			return zero, false, nil
		}
		var zero V
		return zero, false, e
	}

	return c.memCache().Get(full)
}

func (c *HybridCache[V]) SetWithTTL(key string, v V, ttl time.Duration) error {
	full := c.ns.FullKey(key)
	if full == "" {
		return nil
	}

	if c.redisOn() {
		raw, err := c.redisCodec.Encode(v)
		if err != nil {
			return err
		}
		ctx, cancel := context.WithTimeout(context.Background(), defaultRedisOpTimeout)
		defer cancel()
		return c.redis.Set(ctx, full, raw, ttl).Err()
	}

	c.memCache().SetWithTTL(full, v, ttl)
	return nil
}

// Keys returns keys with valid values. In Redis, it returns all matching keys.
func (c *HybridCache[V]) Keys() ([]string, error) {
	keys := make([]string, 0, 1024)
	err := c.ForEachKey(func(key string) error {
		keys = append(keys, key)
		return nil
	})
	return keys, err
}

// ForEachKey visits matching keys without retaining the complete Redis key
// set in the Go heap. Redis-backed caches can outlive the process and may
// contain more entries than an in-process cache capacity, so callers that
// only need to count or inspect keys should use this method instead of Keys.
func (c *HybridCache[V]) ForEachKey(fn func(string) error) error {
	if fn == nil {
		return errors.New("cache key visitor is nil")
	}
	if !c.redisOn() {
		for _, key := range c.memCache().Keys() {
			if err := fn(key); err != nil {
				return err
			}
		}
		return nil
	}

	ctx, cancel := context.WithTimeout(context.Background(), defaultRedisScanTimeout)
	defer cancel()

	var cursor uint64
	for {
		keys, next, err := c.redis.Scan(ctx, cursor, c.ns.MatchPattern(), redisScanBatchSize).Result()
		if err != nil {
			return err
		}
		for _, key := range keys {
			if err := fn(key); err != nil {
				return err
			}
		}
		cursor = next
		if cursor == 0 {
			break
		}
	}
	return nil
}

// deleteMatchingKeys deletes Redis keys one SCAN batch at a time. Keeping the
// scan and delete batches bounded prevents a cache purge from retaining every
// key in memory, while UNLINK keeps large values off the Redis server's main
// command path.
func (c *HybridCache[V]) deleteMatchingKeys(match string) (int, error) {
	ctx, cancel := context.WithTimeout(context.Background(), defaultRedisScanTimeout)
	defer cancel()

	var cursor uint64
	deleted := 0
	for {
		keys, next, err := c.redis.Scan(ctx, cursor, match, redisScanBatchSize).Result()
		if err != nil {
			return deleted, err
		}
		if len(keys) > 0 {
			result, err := c.DeleteMany(keys)
			if err != nil {
				return deleted, err
			}
			for _, ok := range result {
				if ok {
					deleted++
				}
			}
		}
		cursor = next
		if cursor == 0 {
			return deleted, nil
		}
	}
}

func (c *HybridCache[V]) Purge() error {
	if c.redisOn() {
		_, err := c.deleteMatchingKeys(c.ns.MatchPattern())
		return err
	}

	c.memCache().Purge()
	return nil
}

func (c *HybridCache[V]) DeleteByPrefix(prefix string) (int, error) {
	fullPrefix := c.ns.FullKey(prefix)
	if fullPrefix == "" {
		return 0, nil
	}
	if !strings.HasSuffix(fullPrefix, ":") {
		fullPrefix += ":"
	}

	if c.redisOn() {
		return c.deleteMatchingKeys(fullPrefix + "*")
	}

	// In memory, we filter keys and bulk delete.
	allKeys := c.memCache().Keys()
	keys := make([]string, 0, 128)
	for _, k := range allKeys {
		if strings.HasPrefix(k, fullPrefix) {
			keys = append(keys, k)
		}
	}
	if len(keys) == 0 {
		return 0, nil
	}
	res, _ := c.DeleteMany(keys)
	deleted := 0
	for _, ok := range res {
		if ok {
			deleted++
		}
	}
	return deleted, nil
}

// DeleteAll removes every entry in this cache namespace and reports how many
// keys were removed. Unlike Keys followed by DeleteMany, the Redis path keeps
// only one SCAN batch alive at a time.
func (c *HybridCache[V]) DeleteAll() (int, error) {
	if c.redisOn() {
		return c.deleteMatchingKeys(c.ns.MatchPattern())
	}
	keys := c.memCache().Keys()
	if len(keys) == 0 {
		return 0, nil
	}
	result := c.memCache().DeleteMany(keys)
	deleted := 0
	for _, ok := range result {
		if ok {
			deleted++
		}
	}
	return deleted, nil
}

// DeleteMany accepts either fully namespaced keys or raw keys and deletes them.
// It returns a map keyed by fully namespaced keys.
func (c *HybridCache[V]) DeleteMany(keys []string) (map[string]bool, error) {
	res := make(map[string]bool, len(keys))
	if len(keys) == 0 {
		return res, nil
	}

	fullKeys := make([]string, 0, len(keys))
	for _, k := range keys {
		k = c.ns.FullKey(k)
		if k == "" {
			continue
		}
		fullKeys = append(fullKeys, k)
	}
	if len(fullKeys) == 0 {
		return res, nil
	}

	if c.redisOn() {
		ctx, cancel := context.WithTimeout(context.Background(), defaultRedisDelTimeout)
		defer cancel()

		pipe := c.redis.Pipeline()
		cmds := make([]*redis.IntCmd, 0, len(fullKeys))
		for _, k := range fullKeys {
			// UNLINK is non-blocking vs DEL for large key batches.
			cmds = append(cmds, pipe.Unlink(ctx, k))
		}
		_, err := pipe.Exec(ctx)
		if err != nil && !errors.Is(err, redis.Nil) {
			return res, err
		}
		for i, cmd := range cmds {
			deleted := cmd != nil && cmd.Err() == nil && cmd.Val() > 0
			res[fullKeys[i]] = deleted
		}
		return res, nil
	}

	return c.memCache().DeleteMany(fullKeys), nil
}

func (c *HybridCache[V]) Capacity() (mainCacheCapacity int, missingCacheCapacity int) {
	if c.redisOn() {
		return 0, 0
	}
	return c.memCache().Capacity()
}

func (c *HybridCache[V]) Algorithm() (mainCacheAlgorithm string, missingCacheAlgorithm string) {
	if c.redisOn() {
		return "redis", ""
	}
	return c.memCache().Algorithm()
}
