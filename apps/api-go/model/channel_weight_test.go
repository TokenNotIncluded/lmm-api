package model

import "testing"

func TestChannelGetWeightDoesNotWrapLargePersistedValues(t *testing.T) {
	tooLarge := ^uint(0)
	channel := &Channel{Weight: &tooLarge}

	maxInt := int(^uint(0) >> 1)
	if got := channel.GetWeight(); got != maxInt {
		t.Fatalf("GetWeight() = %d, want saturated max int %d", got, maxInt)
	}
}
