/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.
*/

package common

import (
	"testing"
)

func TestRegionPolicyNormalizesAndMatchesCodes(t *testing.T) {
	originalEnabled := IsRegionAccessPolicyEnabled()
	originalCodes := RegionBlockedCountryCodesString()
	t.Cleanup(func() {
		SetRegionAccessPolicyEnabled(originalEnabled)
		_ = SetRegionBlockedCountryCodes(originalCodes)
	})

	if err := SetRegionBlockedCountryCodes(" cn,US,CN "); err != nil {
		t.Fatal(err)
	}
	if got := RegionBlockedCountryCodesString(); got != "CN,US" {
		t.Fatalf("normalized codes = %q, want CN,US", got)
	}
	SetRegionAccessPolicyEnabled(true)
	if !IsRegionBlocked(" us ") || IsRegionBlocked("DE") {
		t.Fatal("region block matching is incorrect")
	}
	SetRegionAccessPolicyEnabled(false)
	if IsRegionBlocked("US") {
		t.Fatal("disabled policy must not block")
	}
}

func TestParseRegionBlockedCountryCodesRejectsEmptyAndInvalidValues(t *testing.T) {
	for _, input := range []string{"", ",", "C", "USA", "CN;US"} {
		if _, err := ParseRegionBlockedCountryCodes(input); err == nil {
			t.Fatalf("ParseRegionBlockedCountryCodes(%q) unexpectedly succeeded", input)
		}
	}
}
