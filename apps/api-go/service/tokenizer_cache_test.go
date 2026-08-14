package service

import (
	"fmt"
	"testing"
)

func TestUnknownModelTokenEncoderCacheIsBounded(t *testing.T) {
	InitTokenEncoders()
	tokenEncoders.Purge()
	for i := 0; i < 10_000; i++ {
		if encoder := getTokenEncoder(fmt.Sprintf("untrusted-model-%d", i)); encoder == nil {
			t.Fatal("fallback encoder is nil")
		}
	}
	maxEntries, _ := tokenEncoders.Capacity()
	if tokenEncoders.Len() > maxEntries {
		t.Fatalf("entry count = %d, max = %d", tokenEncoders.Len(), maxEntries)
	}
}
