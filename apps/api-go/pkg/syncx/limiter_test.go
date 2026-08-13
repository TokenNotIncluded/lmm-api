package syncx

import (
	"testing"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestLimiterFailsFastAtBudget(t *testing.T) {
	limiter := NewLimiter(1)
	release, ok := limiter.TryAcquire()
	require.True(t, ok)
	_, ok = limiter.TryAcquire()
	assert.False(t, ok)
	release()
	assert.Zero(t, limiter.Len())
}
