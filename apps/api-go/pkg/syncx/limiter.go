package syncx

// Limiter is a small fail-fast concurrency budget. It avoids queueing work
// that has already allocated request state when the service is saturated.
type Limiter struct {
	slots chan struct{}
}

func NewLimiter(limit int) *Limiter {
	if limit < 1 {
		limit = 1
	}
	return &Limiter{slots: make(chan struct{}, limit)}
}

func (l *Limiter) TryAcquire() (release func(), ok bool) {
	select {
	case l.slots <- struct{}{}:
		return func() { <-l.slots }, true
	default:
		return func() {}, false
	}
}

func (l *Limiter) Len() int { return len(l.slots) }
