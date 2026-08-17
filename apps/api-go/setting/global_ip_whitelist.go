/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.
*/

package setting

import (
	"encoding/json"
	"fmt"
	"net/netip"
	"strings"
	"sync"
)

const (
	GlobalIPWhitelistEnabledOptionKey = "GlobalIPWhitelistEnabled"
	GlobalIPWhitelistCIDRsOptionKey   = "GlobalIPWhitelistCIDRs"
)

type GlobalIPWhitelistSettings struct {
	Enabled bool
	CIDRs   []string

	prefixes []netip.Prefix
}

var (
	globalIPWhitelistMu sync.RWMutex
	globalIPWhitelist   = GlobalIPWhitelistSettings{
		Enabled:  false,
		CIDRs:    []string{},
		prefixes: []netip.Prefix{},
	}
)

func GetGlobalIPWhitelistSettings() GlobalIPWhitelistSettings {
	globalIPWhitelistMu.RLock()
	defer globalIPWhitelistMu.RUnlock()

	settings := globalIPWhitelist
	settings.CIDRs = append([]string(nil), globalIPWhitelist.CIDRs...)
	settings.prefixes = append([]netip.Prefix(nil), globalIPWhitelist.prefixes...)
	return settings
}

func SetGlobalIPWhitelistEnabled(enabled bool) {
	globalIPWhitelistMu.Lock()
	defer globalIPWhitelistMu.Unlock()
	globalIPWhitelist.Enabled = enabled
}

func UpdateGlobalIPWhitelistCIDRs(value string) error {
	cidrs, prefixes, err := ParseAntiRelayCIDRs(value)
	if err != nil {
		return fmt.Errorf("invalid global IP whitelist: %w", err)
	}

	globalIPWhitelistMu.Lock()
	defer globalIPWhitelistMu.Unlock()
	globalIPWhitelist.CIDRs = cidrs
	globalIPWhitelist.prefixes = prefixes
	return nil
}

func GlobalIPWhitelistCIDRsToJSONString() string {
	settings := GetGlobalIPWhitelistSettings()
	if len(settings.CIDRs) == 0 {
		return "[]"
	}
	encoded, err := json.Marshal(settings.CIDRs)
	if err != nil {
		return "[]"
	}
	return string(encoded)
}

func ValidateGlobalIPWhitelistOption(key, value string) error {
	switch key {
	case GlobalIPWhitelistEnabledOptionKey:
		if value != "true" && value != "false" {
			return fmt.Errorf("%s must be true or false", key)
		}
	case GlobalIPWhitelistCIDRsOptionKey:
		_, _, err := ParseAntiRelayCIDRs(value)
		return err
	}
	return nil
}

func (settings GlobalIPWhitelistSettings) Allows(rawIP string) bool {
	if !settings.Enabled {
		return true
	}
	address, err := netip.ParseAddr(strings.TrimSpace(rawIP))
	if err != nil {
		return false
	}
	address = address.Unmap()
	for _, prefix := range settings.prefixes {
		if prefix.Contains(address) {
			return true
		}
	}
	return false
}
