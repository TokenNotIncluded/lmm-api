package setting

import (
	"net/netip"
	"testing"

	"github.com/stretchr/testify/require"
)

func TestParseAntiRelayCIDRsNormalizesIPsAndDeduplicates(t *testing.T) {
	cidrs, prefixes, err := ParseAntiRelayCIDRs(`[
  " 192.0.2.1 ",
  "192.0.2.1/32",
  "2001:db8::1/64",
  "::ffff:192.0.2.1"
]`)
	require.NoError(t, err)
	require.Equal(t, []string{"192.0.2.1/32", "2001:db8::/64"}, cidrs)
	require.Len(t, prefixes, 2)

	settings := GetAntiRelaySettings()
	require.False(t, settings.IsTrustedProxy(prefixes[0].Addr()), "test address must not be trusted by default")
}

func TestParseAntiRelayCIDRsRejectsUnsafeShapes(t *testing.T) {
	testCases := []string{
		`{"not":"an array"}`,
		`null`,
		`[""]`,
		`["not-an-ip"]`,
		`["192.0.2.1/33"]`,
	}

	for _, value := range testCases {
		t.Run(value, func(t *testing.T) {
			_, _, err := ParseAntiRelayCIDRs(value)
			require.Error(t, err)
		})
	}
}

func TestAntiRelaySettingsUpdatesAreCompiled(t *testing.T) {
	original := GetAntiRelaySettings()
	t.Cleanup(func() {
		SetAntiRelayEnabled(original.Enabled)
		SetAntiRelayRejectProxyHeaders(original.RejectProxyHeaders)
		SetAntiRelayHTTPSOnly(original.HTTPSOnly)
		require.NoError(t, UpdateAntiRelayBlockedCIDRs(marshalAntiRelayCIDRs(original.BlockedCIDRs)))
		require.NoError(t, UpdateAntiRelayTrustedProxyCIDRs(marshalAntiRelayCIDRs(original.TrustedProxyCIDRs)))
	})

	SetAntiRelayEnabled(true)
	SetAntiRelayRejectProxyHeaders(false)
	SetAntiRelayHTTPSOnly(true)
	require.NoError(t, UpdateAntiRelayBlockedCIDRs(`["198.51.100.0/24"]`))
	require.NoError(t, UpdateAntiRelayTrustedProxyCIDRs(`["203.0.113.7"]`))

	updated := GetAntiRelaySettings()
	require.True(t, updated.Enabled)
	require.False(t, updated.RejectProxyHeaders)
	require.True(t, updated.HTTPSOnly)
	require.True(t, updated.IsBlockedPeer(mustAddr(t, "198.51.100.25")))
	require.True(t, updated.IsTrustedProxy(mustAddr(t, "203.0.113.7")))
	require.False(t, updated.IsTrustedProxy(mustAddr(t, "203.0.113.8")))
}

func mustAddr(t *testing.T, value string) netip.Addr {
	t.Helper()
	address, err := netip.ParseAddr(value)
	require.NoError(t, err)
	return address
}
