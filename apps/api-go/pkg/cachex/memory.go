package cachex

import "time"

// MemoryCache is the in-process half of HybridCache. Keeping this interface
// small lets callers choose an entry-count cache or a strict byte-budget cache.
type MemoryCache[V any] interface {
	Get(key string) (V, bool, error)
	SetWithTTL(key string, value V, ttl time.Duration)
	Keys() []string
	DeleteMany(keys []string) map[string]bool
	Purge()
	Capacity() (int, int)
	Algorithm() (string, string)
}
