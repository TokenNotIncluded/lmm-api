package cachex

import "sync"

// FixedMap is a small insertion-ordered map with a hard entry ceiling. It is
// intended for immutable derived values such as compiled regular expressions.
type FixedMap[K comparable, V any] struct {
	mu    sync.Mutex
	max   int
	data  map[K]V
	order []K
}

func NewFixedMap[K comparable, V any](maxEntries int) *FixedMap[K, V] {
	if maxEntries < 1 {
		maxEntries = 1
	}
	return &FixedMap[K, V]{max: maxEntries, data: make(map[K]V, maxEntries), order: make([]K, 0, maxEntries)}
}

func (m *FixedMap[K, V]) Load(key K) (V, bool) {
	m.mu.Lock()
	defer m.mu.Unlock()
	value, ok := m.data[key]
	return value, ok
}

func (m *FixedMap[K, V]) Store(key K, value V) {
	m.mu.Lock()
	defer m.mu.Unlock()
	if _, exists := m.data[key]; exists {
		m.data[key] = value
		return
	}
	if len(m.order) == m.max {
		oldest := m.order[0]
		delete(m.data, oldest)
		copy(m.order, m.order[1:])
		m.order = m.order[:len(m.order)-1]
	}
	m.data[key] = value
	m.order = append(m.order, key)
}

func (m *FixedMap[K, V]) Len() int {
	m.mu.Lock()
	defer m.mu.Unlock()
	return len(m.data)
}
