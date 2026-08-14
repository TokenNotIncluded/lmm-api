// In-memory store + Redis persistence for per-model pricing state.
//
// Concurrency model: single writer / multiple readers.
//
//   - The pricing ticker goroutine is the only writer. It must go through
//     SetState (which takes statesMu.Lock) whenever it replaces the stored
//     ModelState for a model; it must not mutate a stored *ModelState in
//     place outside SetState.
//   - The request path reads via GetMultiplier, the hot path called once per
//     request. It takes statesMu.RLock, copies the Factor float64 out, and
//     releases the lock immediately: no Redis, no allocation, no long-held
//     lock.
//   - GetState returns a defensive COPY of the stored state so callers can
//     never race the writer by mutating the shared pointer.
//
// Redis is a best-effort persistence layer only: it is written through
// SetState and read back on cold start via LoadFromRedis. Every Redis
// failure is logged and swallowed, so Redis being down degrades to
// in-memory-only pricing, never to an error on the request path.
package dynamic_pricing

import (
	"encoding/json"
	"fmt"
	"math"
	"sync"
	"time"

	"github.com/QuantumNous/new-api/common"
	"github.com/QuantumNous/new-api/setting/dynamic_pricing_setting"
)

// stateTTL bounds how long a persisted pricing state survives in Redis. The
// state is fully reconstructible from a few ticks of window data, so 24h is
// plenty: it only needs to bridge a cold start of the pricing goroutine.
const stateTTL = 24 * time.Hour

// stateKey returns the Redis key for a model's persisted pricing state.
func stateKey(model string) string {
	return "dynamic_pricing:state:" + model
}

// redisAvailable reports whether Redis persistence is usable. The canonical
// availability check in this codebase is common.RedisEnabled AND a live
// common.RDB client (see model/subscription.go). The common helpers
// RedisSet/RedisGet dereference common.RDB without a nil check, so callers
// must gate on both flags before calling them.
func redisAvailable() bool {
	return common.RedisEnabled && common.RDB != nil
}

var (
	statesMu sync.RWMutex
	states   = map[string]*ModelState{}
)

// GetState returns a defensive copy of the in-memory state for model. The
// second return value is false when the model has no in-memory state yet.
// Callers must not rely on the returned pointer being shared: it is a copy,
// safe to mutate.
func GetState(model string) (*ModelState, bool) {
	statesMu.RLock()
	defer statesMu.RUnlock()
	st, ok := states[model]
	if !ok {
		return nil, false
	}
	cp := *st
	return &cp, true
}

// SetState stores st as the in-memory state for model and best-effort
// persists it to Redis (TTL stateTTL). It is the single write path; only the
// pricing ticker goroutine should call it. Redis errors are logged via
// common.SysError and never propagated: persistence must never fail the
// caller.
func SetState(model string, st *ModelState) {
	statesMu.Lock()
	states[model] = st
	statesMu.Unlock()

	if !redisAvailable() {
		return
	}
	data, err := json.Marshal(st)
	if err != nil {
		common.SysError(fmt.Sprintf("dynamic_pricing: marshal state for model %s: %s", model, err.Error()))
		return
	}
	if err := common.RedisSet(stateKey(model), string(data), stateTTL); err != nil {
		common.SysError(fmt.Sprintf("dynamic_pricing: persist state for model %s: %s", model, err.Error()))
	}
}

// GetMultiplier returns the current dynamic multiplier for model: the stored
// Factor when present and positive, otherwise 1.0 (base price). This is the
// hot path, called once per request; it only reads the Factor float64 under
// RLock and touches neither Redis nor the rest of the state.
func GetMultiplier(model string) float64 {
	// Feature master switch: while the admin has disabled dynamic pricing the
	// ticker is a no-op, so in-memory Factors freeze at their last value. The
	// request path must not keep charging those stale multipliers, so gate on
	// the setting here (covers every caller). IsEnabled reads a single bool,
	// no map copies, so the per-request cost stays negligible.
	if !dynamic_pricing_setting.IsEnabled() {
		return 1.0
	}
	statesMu.RLock()
	st, ok := states[model]
	var f float64
	if ok && st != nil {
		f = st.Factor
	}
	statesMu.RUnlock()
	if f >= 1 && !math.IsNaN(f) && !math.IsInf(f, 0) {
		maxFactor := dynamic_pricing_setting.GetMaxFactor()
		if f > maxFactor {
			return maxFactor
		}
		return f
	}
	return 1.0
}

// LoadFromRedis fetches the persisted state for model from Redis (used by
// the ticker on a cold start, when the in-memory lookup misses), and on
// success also populates the in-memory store via SetState. A missing key,
// Redis being unavailable, or any parse failure returns (nil, false) and is
// treated as "no persisted state" rather than an error.
func LoadFromRedis(model string) (*ModelState, bool) {
	if !redisAvailable() {
		return nil, false
	}
	raw, err := common.RedisGet(stateKey(model))
	if err != nil {
		return nil, false
	}
	var st ModelState
	if err := json.Unmarshal([]byte(raw), &st); err != nil {
		common.SysError(fmt.Sprintf("dynamic_pricing: unmarshal state for model %s: %s", model, err.Error()))
		return nil, false
	}
	setting := dynamic_pricing_setting.GetSetting()
	if err := setting.Validate(); err != nil {
		return nil, false
	}
	ClampState(&st, setting.MaxFactor)
	SetState(model, &st)
	// Return a defensive copy: the stored pointer must only be mutated under
	// SetState (the ticker does Tick on the returned value before the next
	// SetState), so handing out the same pointer would race GetState /
	// GetMultiplier readers.
	cp := st
	return &cp, true
}

// AllModels returns the model names currently held in the in-memory store
// (for the status API). Order is unspecified.
func AllModels() []string {
	statesMu.RLock()
	defer statesMu.RUnlock()
	keys := make([]string, 0, len(states))
	for k := range states {
		keys = append(keys, k)
	}
	return keys
}
