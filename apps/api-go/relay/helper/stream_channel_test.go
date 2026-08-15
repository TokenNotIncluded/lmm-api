package helper

import (
	"context"
	"testing"
	"time"
)

func TestSendCtx(t *testing.T) {
	t.Run("delivers value", func(t *testing.T) {
		ctx := context.Background()
		ch := make(chan string, 1)
		if !SendCtx(ctx, ch, "chunk") {
			t.Fatal("SendCtx returned false for an available channel")
		}
		if got := <-ch; got != "chunk" {
			t.Fatalf("received %q, want chunk", got)
		}
	})

	t.Run("stops after cancellation", func(t *testing.T) {
		ctx, cancel := context.WithCancel(context.Background())
		defer cancel()
		ch := make(chan string)
		result := make(chan bool, 1)
		go func() {
			result <- SendCtx(ctx, ch, "chunk")
		}()
		cancel()

		select {
		case sent := <-result:
			if sent {
				t.Fatal("SendCtx reported a send after cancellation")
			}
		case <-time.After(time.Second):
			t.Fatal("SendCtx did not unblock after cancellation")
		}
	})
}
