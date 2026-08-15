package helper

import "context"

// SendCtx sends a stream value unless the request context has been canceled.
// It is intended for producer goroutines feeding an unbuffered relay channel;
// a disconnected client must not leave those goroutines blocked forever.
func SendCtx[T any](ctx context.Context, ch chan<- T, value T) bool {
	select {
	case ch <- value:
		return true
	case <-ctx.Done():
		return false
	}
}
