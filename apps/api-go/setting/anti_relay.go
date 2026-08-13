package setting

import (
	"encoding/json"
	"fmt"
	"net/netip"
	"strings"
	"sync"
)

const (
	AntiRelayEnabledOptionKey            = "AntiRelayEnabled"
	AntiRelayRejectProxyHeadersOptionKey = "AntiRelayRejectProxyHeadersEnabled"
	AntiRelayHTTPSOnlyOptionKey          = "AntiRelayHTTPSOnlyEnabled"
	AntiRelayBlockedCIDRsOptionKey       = "AntiRelayBlockedCIDRs"
	AntiRelayTrustedProxyCIDRsOptionKey  = "AntiRelayTrustedProxyCIDRs"
	antiRelayMaxCIDRs                    = 256
	antiRelayMaxCIDRListBytes            = 16 * 1024
)

var (
	antiRelayDefaultTrustedProxyCIDRs = []string{
		"127.0.0.1/32",
		"::1/128",
	}
	antiRelayDefaultTrustedProxyPrefixes = []netip.Prefix{
		netip.MustParsePrefix("127.0.0.1/32"),
		netip.MustParsePrefix("::1/128"),
	}
	antiRelaySettingsMu sync.RWMutex
	antiRelaySettings   = AntiRelaySettings{
		Enabled:              false,
		RejectProxyHeaders:   true,
		HTTPSOnly:            false,
		BlockedCIDRs:         []string{},
		TrustedProxyCIDRs:    append([]string(nil), antiRelayDefaultTrustedProxyCIDRs...),
		blockedPrefixes:      []netip.Prefix{},
		trustedProxyPrefixes: append([]netip.Prefix(nil), antiRelayDefaultTrustedProxyPrefixes...),
	}
)

// AntiRelaySettings is the process-wide request ingress policy. The public
// CIDR fields are kept in their canonical string form for the settings UI;
// the private prefixes are compiled once so every request can evaluate them
// without reparsing administrator input.
type AntiRelaySettings struct {
	Enabled            bool
	RejectProxyHeaders bool
	HTTPSOnly          bool
	BlockedCIDRs       []string
	TrustedProxyCIDRs  []string

	blockedPrefixes      []netip.Prefix
	trustedProxyPrefixes []netip.Prefix
}

func GetAntiRelaySettings() AntiRelaySettings {
	antiRelaySettingsMu.RLock()
	defer antiRelaySettingsMu.RUnlock()

	settings := antiRelaySettings
	settings.BlockedCIDRs = append([]string(nil), antiRelaySettings.BlockedCIDRs...)
	settings.TrustedProxyCIDRs = append([]string(nil), antiRelaySettings.TrustedProxyCIDRs...)
	settings.blockedPrefixes = append([]netip.Prefix(nil), antiRelaySettings.blockedPrefixes...)
	settings.trustedProxyPrefixes = append([]netip.Prefix(nil), antiRelaySettings.trustedProxyPrefixes...)
	return settings
}

func SetAntiRelayEnabled(enabled bool) {
	antiRelaySettingsMu.Lock()
	defer antiRelaySettingsMu.Unlock()
	antiRelaySettings.Enabled = enabled
}

func SetAntiRelayRejectProxyHeaders(enabled bool) {
	antiRelaySettingsMu.Lock()
	defer antiRelaySettingsMu.Unlock()
	antiRelaySettings.RejectProxyHeaders = enabled
}

func SetAntiRelayHTTPSOnly(enabled bool) {
	antiRelaySettingsMu.Lock()
	defer antiRelaySettingsMu.Unlock()
	antiRelaySettings.HTTPSOnly = enabled
}

func UpdateAntiRelayBlockedCIDRs(value string) error {
	cidrs, prefixes, err := ParseAntiRelayCIDRs(value)
	if err != nil {
		return fmt.Errorf("invalid anti-relay blocked CIDRs: %w", err)
	}

	antiRelaySettingsMu.Lock()
	defer antiRelaySettingsMu.Unlock()
	antiRelaySettings.BlockedCIDRs = cidrs
	antiRelaySettings.blockedPrefixes = prefixes
	return nil
}

func UpdateAntiRelayTrustedProxyCIDRs(value string) error {
	cidrs, prefixes, err := ParseAntiRelayCIDRs(value)
	if err != nil {
		return fmt.Errorf("invalid anti-relay trusted proxy CIDRs: %w", err)
	}

	antiRelaySettingsMu.Lock()
	defer antiRelaySettingsMu.Unlock()
	antiRelaySettings.TrustedProxyCIDRs = cidrs
	antiRelaySettings.trustedProxyPrefixes = prefixes
	return nil
}

func AntiRelayBlockedCIDRsToJSONString() string {
	settings := GetAntiRelaySettings()
	return marshalAntiRelayCIDRs(settings.BlockedCIDRs)
}

func AntiRelayTrustedProxyCIDRsToJSONString() string {
	settings := GetAntiRelaySettings()
	return marshalAntiRelayCIDRs(settings.TrustedProxyCIDRs)
}

func ValidateAntiRelayOption(key string, value string) error {
	switch key {
	case AntiRelayEnabledOptionKey, AntiRelayRejectProxyHeadersOptionKey, AntiRelayHTTPSOnlyOptionKey:
		if value != "true" && value != "false" {
			return fmt.Errorf("%s must be true or false", key)
		}
	case AntiRelayBlockedCIDRsOptionKey, AntiRelayTrustedProxyCIDRsOptionKey:
		_, _, err := ParseAntiRelayCIDRs(value)
		return err
	}
	return nil
}

// ParseAntiRelayCIDRs accepts a JSON string array. Each entry may be an IP or
// a CIDR; bare IPs are normalized to host prefixes so matching is consistent
// for IPv4, IPv6, and IPv4-mapped IPv6 addresses.
func ParseAntiRelayCIDRs(value string) ([]string, []netip.Prefix, error) {
	raw := strings.TrimSpace(value)
	if len(raw) > antiRelayMaxCIDRListBytes {
		return nil, nil, fmt.Errorf("CIDR list cannot exceed %d bytes", antiRelayMaxCIDRListBytes)
	}
	if raw == "" {
		return []string{}, []netip.Prefix{}, nil
	}

	var entries []string
	if err := json.Unmarshal([]byte(raw), &entries); err != nil {
		return nil, nil, fmt.Errorf("CIDR list must be a JSON array of strings: %w", err)
	}
	if entries == nil {
		return nil, nil, fmt.Errorf("CIDR list must be a JSON array, not null")
	}
	if len(entries) > antiRelayMaxCIDRs {
		return nil, nil, fmt.Errorf("CIDR list cannot contain more than %d entries", antiRelayMaxCIDRs)
	}

	canonical := make([]string, 0, len(entries))
	prefixes := make([]netip.Prefix, 0, len(entries))
	seen := make(map[string]struct{}, len(entries))
	for index, entry := range entries {
		entry = strings.TrimSpace(entry)
		if entry == "" {
			return nil, nil, fmt.Errorf("CIDR entry %d is empty", index+1)
		}
		prefix, err := parseAntiRelayPrefix(entry)
		if err != nil {
			return nil, nil, fmt.Errorf("CIDR entry %d is invalid: %w", index+1, err)
		}
		canonicalEntry := prefix.String()
		if _, exists := seen[canonicalEntry]; exists {
			continue
		}
		seen[canonicalEntry] = struct{}{}
		canonical = append(canonical, canonicalEntry)
		prefixes = append(prefixes, prefix)
	}

	return canonical, prefixes, nil
}

func (settings AntiRelaySettings) IsBlockedPeer(address netip.Addr) bool {
	address = address.Unmap()
	for _, prefix := range settings.blockedPrefixes {
		if prefix.Contains(address) {
			return true
		}
	}
	return false
}

func (settings AntiRelaySettings) IsTrustedProxy(address netip.Addr) bool {
	address = address.Unmap()
	for _, prefix := range settings.trustedProxyPrefixes {
		if prefix.Contains(address) {
			return true
		}
	}
	return false
}

func parseAntiRelayPrefix(value string) (netip.Prefix, error) {
	if prefix, err := netip.ParsePrefix(value); err == nil {
		address := prefix.Addr()
		if address.Zone() != "" {
			return netip.Prefix{}, fmt.Errorf("zones are not supported")
		}
		unmapped := address.Unmap()
		if unmapped != address {
			bits := prefix.Bits()
			if bits < 96 {
				return netip.Prefix{}, fmt.Errorf("invalid IPv4-mapped IPv6 prefix")
			}
			return netip.PrefixFrom(unmapped, bits-96).Masked(), nil
		}
		return prefix.Masked(), nil
	}

	address, err := netip.ParseAddr(value)
	if err != nil {
		return netip.Prefix{}, fmt.Errorf("must be an IP address or CIDR")
	}
	if address.Zone() != "" {
		return netip.Prefix{}, fmt.Errorf("zones are not supported")
	}
	address = address.Unmap()
	return netip.PrefixFrom(address, address.BitLen()).Masked(), nil
}

func marshalAntiRelayCIDRs(cidrs []string) string {
	if len(cidrs) == 0 {
		return "[]"
	}
	encoded, err := json.Marshal(cidrs)
	if err != nil {
		return "[]"
	}
	return string(encoded)
}
