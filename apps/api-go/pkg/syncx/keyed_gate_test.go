package syncx

import (
	"context"
	"testing"
	"time"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestKeyedGateBoundsDistinctKeys(t *testing.T) {
	gate := NewKeyedGate(1)
	release, ok := gate.Acquire(context.Background(), "first")
	require.True(t, ok)
	_, ok = gate.Acquire(context.Background(), "second")
	assert.False(t, ok)
	assert.Equal(t, 1, gate.Len())
	release()
	assert.Zero(t, gate.Len())
}

func TestKeyedGateCoalescesSameKey(t *testing.T) {
	gate := NewKeyedGate(1)
	release, ok := gate.Acquire(context.Background(), "same")
	require.True(t, ok)
	acquired := make(chan bool, 1)
	go func() {
		nextRelease, nextOK := gate.Acquire(context.Background(), "same")
		if nextOK {
			nextRelease()
		}
		acquired <- nextOK
	}()
	select {
	case <-acquired:
		t.Fatal("same-key waiter must wait for the owner")
	case <-time.After(10 * time.Millisecond):
	}
	release()
	assert.True(t, <-acquired)
}
