package syncx

import (
	"testing"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestKeyedTryLockerRejectsDuplicateAndCleansIdleKey(t *testing.T) {
	locker := NewKeyedTryLocker[int]()
	release, ok := locker.TryLock(7)
	require.True(t, ok)
	_, ok = locker.TryLock(7)
	assert.False(t, ok)
	assert.Equal(t, 1, locker.Len())
	release()
	assert.Zero(t, locker.Len())
}

func TestKeyedTryLockerAllowsDifferentKeys(t *testing.T) {
	locker := NewKeyedTryLocker[int]()
	first, firstOK := locker.TryLock(1)
	second, secondOK := locker.TryLock(2)
	require.True(t, firstOK)
	require.True(t, secondOK)
	first()
	second()
	assert.Zero(t, locker.Len())
}
