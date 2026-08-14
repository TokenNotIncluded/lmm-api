package syncx

import "sync"

type tryLockEntry struct {
	token chan struct{}
	refs  int
}

// KeyedTryLocker provides fail-fast per-key exclusion and removes idle keys.
// It is suitable for user/order operations where duplicate concurrent work
// should be rejected instead of queued.
type KeyedTryLocker[K comparable] struct {
	mu      sync.Mutex
	entries map[K]*tryLockEntry
}

func NewKeyedTryLocker[K comparable]() *KeyedTryLocker[K] {
	return &KeyedTryLocker[K]{entries: make(map[K]*tryLockEntry)}
}

func (l *KeyedTryLocker[K]) TryLock(key K) (release func(), ok bool) {
	l.mu.Lock()
	entry := l.entries[key]
	if entry == nil {
		entry = &tryLockEntry{token: make(chan struct{}, 1)}
		l.entries[key] = entry
	}
	entry.refs++
	l.mu.Unlock()

	select {
	case entry.token <- struct{}{}:
		var once sync.Once
		return func() {
			once.Do(func() {
				<-entry.token
				l.unref(key, entry)
			})
		}, true
	default:
		l.unref(key, entry)
		return func() {}, false
	}
}

func (l *KeyedTryLocker[K]) unref(key K, entry *tryLockEntry) {
	l.mu.Lock()
	defer l.mu.Unlock()
	entry.refs--
	if entry.refs == 0 && l.entries[key] == entry {
		delete(l.entries, key)
	}
}

func (l *KeyedTryLocker[K]) Len() int {
	l.mu.Lock()
	defer l.mu.Unlock()
	return len(l.entries)
}
