package common

import (
	"sync"
	"time"

	"github.com/QuantumNous/new-api/pkg/cachex"
)

const (
	rateLimitMaxKeys  = 65_536
	rateLimitMaxBytes = 8 << 20
)

type rateWindow struct {
	Count     int
	StartedAt time.Time
}

// InMemoryRateLimiter mirrors the fixed-window Redis limiter while keeping a
// hard budget for key cardinality and bytes. Each key costs O(1) memory even
// when an administrator configures a very large request limit.
type InMemoryRateLimiter struct {
	initOnce sync.Once
	store    *cachex.ByteCache[rateWindow]
}

func (l *InMemoryRateLimiter) Init(_ time.Duration) {
	l.initOnce.Do(func() {
		l.store = cachex.NewByteCache[rateWindow](rateLimitMaxKeys, rateLimitMaxBytes, func(key string, _ rateWindow) int64 {
			return int64(len(key) + 40)
		})
	})
}

// Request records one request and reports whether it is within the limit.
// The duration parameter is in seconds.
func (l *InMemoryRateLimiter) Request(key string, maxRequestNum int, duration int64) bool {
	if maxRequestNum == 0 {
		return true
	}
	if maxRequestNum < 0 || duration <= 0 {
		return false
	}
	l.Init(0)
	now := time.Now()
	window := time.Duration(duration) * time.Second
	allowed := false
	state, stored := l.store.Compute(key, window, func(current rateWindow, found bool) (rateWindow, bool) {
		if !found || now.Sub(current.StartedAt) >= window {
			current = rateWindow{StartedAt: now}
		}
		if current.Count < maxRequestNum {
			current.Count++
			allowed = true
		}
		return current, true
	})
	return stored && allowed && state.Count > 0
}

// Check reports whether a request would be allowed without recording it.
// The duration parameter is in seconds.
func (l *InMemoryRateLimiter) Check(key string, maxRequestNum int, duration int64) bool {
	if maxRequestNum == 0 {
		return true
	}
	if maxRequestNum < 0 || duration <= 0 {
		return false
	}
	l.Init(0)
	state, found := l.store.Load(key)
	return !found || time.Since(state.StartedAt) >= time.Duration(duration)*time.Second || state.Count < maxRequestNum
}
