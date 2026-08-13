package cachex

import (
	"testing"

	"github.com/stretchr/testify/assert"
)

func TestFixedMapEvictsOldestEntry(t *testing.T) {
	cache := NewFixedMap[string, int](2)
	cache.Store("a", 1)
	cache.Store("b", 2)
	cache.Store("c", 3)
	_, found := cache.Load("a")
	assert.False(t, found)
	assert.Equal(t, 2, cache.Len())
}
