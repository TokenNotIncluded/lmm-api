/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.
*/

package common

import (
	"errors"
	"sort"
	"strings"
	"sync"
)

const (
	RegionAccessPolicyEnabledOptionKey = "RegionAccessPolicyEnabled"
	RegionBlockedCountryCodesOptionKey = "RegionBlockedCountryCodes"
	defaultRegionBlockedCountryCode    = "CN"
	maxRegionBlockedCountryCodes       = 64
)

var (
	regionPolicyMu            sync.RWMutex
	regionAccessPolicyEnabled = true
	regionBlockedCountryCodes = []string{defaultRegionBlockedCountryCode}
)

// IsRegionAccessPolicyEnabled reports whether the edge auth_request should
// enforce the configured country block. Reads are synchronized because the
// value can be changed through the administrator settings API at runtime.
func IsRegionAccessPolicyEnabled() bool {
	regionPolicyMu.RLock()
	defer regionPolicyMu.RUnlock()
	return regionAccessPolicyEnabled
}

func SetRegionAccessPolicyEnabled(enabled bool) {
	regionPolicyMu.Lock()
	regionAccessPolicyEnabled = enabled
	regionPolicyMu.Unlock()
}

func RegionBlockedCountryCodes() []string {
	regionPolicyMu.RLock()
	defer regionPolicyMu.RUnlock()
	return append([]string(nil), regionBlockedCountryCodes...)
}

func RegionBlockedCountryCodesString() string {
	return strings.Join(RegionBlockedCountryCodes(), ",")
}

// ParseRegionBlockedCountryCodes accepts a compact comma-separated ISO
// country-code list. Empty values are rejected so turning the policy on can
// never silently mean "block nothing"; operators can disable the policy
// explicitly instead.
func ParseRegionBlockedCountryCodes(raw string) ([]string, error) {
	parts := strings.Split(raw, ",")
	seen := make(map[string]struct{}, len(parts))
	codes := make([]string, 0, len(parts))
	for _, part := range parts {
		code := strings.ToUpper(strings.TrimSpace(part))
		if code == "" {
			continue
		}
		if len(code) != 2 || code[0] < 'A' || code[0] > 'Z' || code[1] < 'A' || code[1] > 'Z' {
			return nil, errors.New("blocked country codes must be two-letter ISO codes")
		}
		if _, exists := seen[code]; exists {
			continue
		}
		seen[code] = struct{}{}
		codes = append(codes, code)
		if len(codes) > maxRegionBlockedCountryCodes {
			return nil, errors.New("too many blocked country codes")
		}
	}
	if len(codes) == 0 {
		return nil, errors.New("at least one blocked country code is required")
	}
	sort.Strings(codes)
	return codes, nil
}

func SetRegionBlockedCountryCodes(raw string) error {
	codes, err := ParseRegionBlockedCountryCodes(raw)
	if err != nil {
		return err
	}
	regionPolicyMu.Lock()
	regionBlockedCountryCodes = codes
	regionPolicyMu.Unlock()
	return nil
}

func IsRegionBlocked(countryCode string) bool {
	if !IsRegionAccessPolicyEnabled() {
		return false
	}
	code := strings.ToUpper(strings.TrimSpace(countryCode))
	if code == "" {
		return false
	}
	regionPolicyMu.RLock()
	defer regionPolicyMu.RUnlock()
	for _, blocked := range regionBlockedCountryCodes {
		if code == blocked {
			return true
		}
	}
	return false
}
