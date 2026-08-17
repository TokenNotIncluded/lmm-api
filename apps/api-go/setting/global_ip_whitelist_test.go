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
	"testing"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestGlobalIPWhitelistMatchesAddressesAndCIDRs(t *testing.T) {
	original := GetGlobalIPWhitelistSettings()
	t.Cleanup(func() {
		encoded, err := json.Marshal(original.CIDRs)
		require.NoError(t, err)
		require.NoError(t, UpdateGlobalIPWhitelistCIDRs(string(encoded)))
		SetGlobalIPWhitelistEnabled(original.Enabled)
	})

	require.NoError(t, UpdateGlobalIPWhitelistCIDRs(`[
		"203.0.113.8",
		"198.51.100.0/24",
		"2001:db8::/48"
	]`))
	SetGlobalIPWhitelistEnabled(true)
	policy := GetGlobalIPWhitelistSettings()

	assert.True(t, policy.Allows("203.0.113.8"))
	assert.True(t, policy.Allows("198.51.100.42"))
	assert.True(t, policy.Allows("2001:db8::10"))
	assert.False(t, policy.Allows("203.0.113.9"))
	assert.False(t, policy.Allows("not-an-ip"))
}

func TestDisabledGlobalIPWhitelistAllowsEveryClient(t *testing.T) {
	policy := GlobalIPWhitelistSettings{Enabled: false}
	assert.True(t, policy.Allows("not-an-ip"))
}
