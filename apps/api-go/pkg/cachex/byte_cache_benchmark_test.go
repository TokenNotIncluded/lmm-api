package cachex

import (
	"strconv"
	"testing"
	"time"
)

func BenchmarkByteCacheBoundedSet(b *testing.B) {
	cache := NewByteCache[string](256, 1<<20, func(key, value string) int64 {
		return int64(len(key) + len(value))
	})
	b.ReportAllocs()
	for i := 0; b.Loop(); i++ {
		cache.SetWithTTL(strconv.Itoa(i), "value", time.Minute)
	}
	if cache.Len() > 256 || cache.Bytes() > 1<<20 {
		b.Fatal("cache exceeded its budget")
	}
}
