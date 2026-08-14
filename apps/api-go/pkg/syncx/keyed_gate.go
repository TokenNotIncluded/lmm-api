package syncx

import (
	"context"
	"sync"
)

type gateEntry struct {
	done chan struct{}
}

// KeyedGate coalesces work for the same key while bounding the number of
// distinct in-flight keys. A high-cardinality burst therefore cannot grow the
// coordination map without limit.
type KeyedGate struct {
	mu      sync.Mutex
	entries map[string]*gateEntry
	maxKeys int
}

func NewKeyedGate(maxKeys int) *KeyedGate {
	if maxKeys < 1 {
		maxKeys = 1
	}
	return &KeyedGate{entries: make(map[string]*gateEntry, maxKeys), maxKeys: maxKeys}
}

// Acquire returns the ownership release function. ok=false means the context
// ended or the distinct-key budget was full. Release is always idempotent.
func (g *KeyedGate) Acquire(ctx context.Context, key string) (release func(), ok bool) {
	if key == "" {
		return func() {}, true
	}
	for {
		g.mu.Lock()
		entry, exists := g.entries[key]
		if !exists {
			if len(g.entries) >= g.maxKeys {
				g.mu.Unlock()
				return func() {}, false
			}
			entry = &gateEntry{done: make(chan struct{})}
			g.entries[key] = entry
			g.mu.Unlock()

			var once sync.Once
			return func() {
				once.Do(func() {
					g.mu.Lock()
					if current, found := g.entries[key]; found && current == entry {
						delete(g.entries, key)
						close(entry.done)
					}
					g.mu.Unlock()
				})
			}, true
		}
		wait := entry.done
		g.mu.Unlock()

		select {
		case <-wait:
		case <-ctx.Done():
			return func() {}, false
		}
	}
}

func (g *KeyedGate) Len() int {
	g.mu.Lock()
	defer g.mu.Unlock()
	return len(g.entries)
}
