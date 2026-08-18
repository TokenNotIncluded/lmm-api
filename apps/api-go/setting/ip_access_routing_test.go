/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.
*/

package setting

import (
	"strings"
	"testing"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func withIPAccessRoutingRules(t *testing.T, rules string) {
	t.Helper()
	original := GetIPAccessRoutingRules()
	require.NoError(t, UpdateIPAccessRoutingRules(rules))
	t.Cleanup(func() {
		require.NoError(t, UpdateIPAccessRoutingRules(original))
	})
}

func TestDefaultIPAccessRoutingRulesRejectChina(t *testing.T) {
	withIPAccessRoutingRules(t, DefaultIPAccessRoutingRules)

	action, line, err := EvaluateIPAccessRoute(IPAccessRouteRequest{
		ClientIP:        "203.0.113.8",
		CountryCode:     "CN",
		L4Protocol:      "tcp",
		DestinationPort: 443,
	})
	require.NoError(t, err)
	assert.Equal(t, IPAccessRouteReject, action)
	assert.Equal(t, 2, line)

	action, line, err = EvaluateIPAccessRoute(IPAccessRouteRequest{
		ClientIP:        "198.51.100.8",
		CountryCode:     "US",
		L4Protocol:      "tcp",
		DestinationPort: 443,
	})
	require.NoError(t, err)
	assert.Equal(t, IPAccessRouteDirect, action)
	assert.Zero(t, line)
}

func TestIPAccessRoutingUsesFirstMatchingRule(t *testing.T) {
	withIPAccessRoutingRules(t, `
# Trusted management endpoints
dip(45.59.187.63, 2001:db8::9) && l4proto(tcp) && dport(443) -> direct
dip(geoip:cn) -> reject
dip(geoip:private) -> direct
`)

	tests := []struct {
		name    string
		request IPAccessRouteRequest
		action  IPAccessRouteAction
		line    int
	}{
		{
			name: "specific China IP is allowed before the country rule",
			request: IPAccessRouteRequest{
				ClientIP: "45.59.187.63", CountryCode: "CN", L4Protocol: "tcp", DestinationPort: 443,
			},
			action: IPAccessRouteDirect,
			line:   2,
		},
		{
			name: "specific IP does not match a different destination port",
			request: IPAccessRouteRequest{
				ClientIP: "45.59.187.63", CountryCode: "CN", L4Protocol: "tcp", DestinationPort: 80,
			},
			action: IPAccessRouteReject,
			line:   3,
		},
		{
			name: "private address matches geoip private",
			request: IPAccessRouteRequest{
				ClientIP: "10.10.0.4", CountryCode: "US", L4Protocol: "tcp", DestinationPort: 443,
			},
			action: IPAccessRouteDirect,
			line:   4,
		},
	}

	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			action, line, err := EvaluateIPAccessRoute(test.request)
			require.NoError(t, err)
			assert.Equal(t, test.action, action)
			assert.Equal(t, test.line, line)
		})
	}
}

func TestIPAccessRoutingFailsClosedWhenRequiredEdgeDataIsMissing(t *testing.T) {
	withIPAccessRoutingRules(t, `
dip(192.0.2.10) -> direct
dip(geoip:cn) -> reject
`)

	action, line, err := EvaluateIPAccessRoute(IPAccessRouteRequest{
		ClientIP: "192.0.2.10",
	})
	require.NoError(t, err)
	assert.Equal(t, IPAccessRouteDirect, action)
	assert.Equal(t, 1, line)

	_, line, err = EvaluateIPAccessRoute(IPAccessRouteRequest{
		ClientIP: "192.0.2.11",
	})
	require.ErrorContains(t, err, "edge country is unavailable")
	assert.Equal(t, 2, line)

	withIPAccessRoutingRules(t, "dip(192.0.2.0/24) && dport(443) -> direct")
	_, line, err = EvaluateIPAccessRoute(IPAccessRouteRequest{ClientIP: "192.0.2.20", L4Protocol: "tcp"})
	require.ErrorContains(t, err, "destination port is unavailable")
	assert.Equal(t, 1, line)
}

func TestParseIPAccessRoutingRulesValidatesDaedSubset(t *testing.T) {
	valid := []string{
		"dip(203.0.113.8) -> direct",
		"dip(203.0.113.0/24, 2001:db8::/32) -> reject",
		"dip(geoip:cn, geoip:private) && l4proto(tcp) && dport(80, 443) -> reject",
		"# comment\ndip(::ffff:192.0.2.4) -> direct # inline comment",
	}
	for _, rules := range valid {
		_, err := ParseIPAccessRoutingRules(rules)
		require.NoError(t, err, rules)
	}

	invalid := map[string]string{
		"":                                           "at least one rule",
		"# comments only":                            "at least one rule",
		"dip(geoip:china) -> reject":                 "invalid geoip",
		"dip(not-an-ip) -> direct":                   "invalid dip value",
		"dip(192.0.2.1) -> proxy":                    "action must be direct or reject",
		"domain(example.com) -> direct":              "not available for inbound HTTP routing",
		"pname(nginx) -> direct":                     "not available for inbound HTTP routing",
		"l4proto(tcp) -> direct":                     "must include dip()",
		"dip(192.0.2.1) && l4proto(udp) -> direct":   "supports only l4proto(tcp)",
		"dip(192.0.2.1) && dport(0) -> direct":       "between 1 and 65535",
		"dip(192.0.2.1) && dip(192.0.2.2) -> direct": "duplicate dip()",
	}
	for rules, message := range invalid {
		_, err := ParseIPAccessRoutingRules(rules)
		require.ErrorContains(t, err, message, rules)
	}

	_, err := ParseIPAccessRoutingRules(strings.Repeat("#", maxIPAccessRoutingRulesBytes+1))
	require.ErrorContains(t, err, "cannot exceed")
}
