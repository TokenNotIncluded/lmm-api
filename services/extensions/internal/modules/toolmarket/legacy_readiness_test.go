// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package toolmarket

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"
)

type readinessAuthority struct {
	*marketMetaFixture
	*executionCredits
	ack LegacyCapabilities
}

func (a *readinessAuthority) LegacyReadiness() LegacyCapabilities {
	r := a.ack
	r.Features = append([]string(nil), r.Features...)
	return r
}
func (a *readinessAuthority) ListLegacyMarketOAuthClients(context.Context, string, LegacyMarketOAuthClientsRequest) (LegacyMarketOAuthClients, error) {
	return LegacyMarketOAuthClients{}, ErrUnavailable
}

type readinessBuiltin struct {
	*BuiltinDispatcher
	ready bool
}

func (b *readinessBuiltin) LegacyBuiltinReady() bool { return b.ready }
func TestLegacyReadinessRejectsMissingDuplicateAndWrongUnit(t *testing.T) {
	good := LegacyCapabilities{ProtocolMajor: 2, Unit: "credit_500k_usd", Features: append([]string(nil), legacyRequiredFeatures...)}
	if ValidateLegacyCapabilities(good, false) != nil {
		t.Fatal("current complete Core receipt rejected")
	}
	if ValidateLegacyCapabilities(good, true) == nil {
		t.Fatal("OAuth falsely ready without actor and issuer ACK")
	}
	for _, field := range []string{"protocol", "unit", "duplicate", "missing", "nullfeatures"} {
		bad := good
		bad.Features = append([]string(nil), good.Features...)
		switch field {
		case "protocol":
			bad.ProtocolMajor = 1
		case "unit":
			bad.Unit = "usd_micros"
		case "duplicate":
			bad.Features = append(bad.Features, "future", "future")
		case "missing":
			bad.Features = bad.Features[1:]
		case "nullfeatures":
			bad.Features = nil
		}
		if ValidateLegacyCapabilities(bad, false) == nil {
			t.Fatal(field)
		}
	}
	good.Features = append(good.Features, "actors.oauth", "oauth.enabled", "future.feature")
	if ValidateLegacyCapabilities(good, true) != nil {
		t.Fatal("compatible added features rejected")
	}
	for _, raw := range []string{`{"protocol_major":2,"unit":"credit_500k_usd"}`, `{"protocol_major":2,"unit":"credit_500k_usd","features":null}`, `{"protocol_major":2,"protocol_major":1,"unit":"credit_500k_usd","features":[]}`} {
		var r LegacyCapabilities
		if json.Unmarshal([]byte(raw), &r) == nil {
			t.Fatal("unknown/missing receipt accepted", raw)
		}
	}
}
func TestLegacyReadinessNeedsCoreAckActualMountAndBuiltinAck(t *testing.T) {
	m, meta, _ := metaConsumerFixture(t)
	a := &readinessAuthority{marketMetaFixture: meta, executionCredits: &executionCredits{}}
	m.authority = a
	opts := LegacyMarketMCPOptions{Issuer: "https://example.test/api/oauth2/market", Resource: "https://example.test/mcp/market", ResourceMetadataURL: "https://example.test/.well-known/oauth-protected-resource/mcp/market"}
	mux := http.NewServeMux()
	if m.LegacyCapabilities() != (LegacyRuntimeCapabilities{}) {
		t.Fatal("configuration fabricated readiness")
	}
	if m.MountLegacyMarketMCP(mux, opts) != ErrUnavailable {
		t.Fatal("mounted before Core ACK")
	}
	handler := m.LegacyMarketMCPHandler(opts)
	w := httptest.NewRecorder()
	handler.ServeHTTP(w, httptest.NewRequest("POST", "/mcp/market", nil))
	if w.Code != 503 {
		t.Fatal("direct construction bypassed Core ACK", w.Code)
	}
	a.ack = LegacyCapabilities{ProtocolMajor: 2, Unit: "credit_500k_usd", Features: append([]string(nil), legacyRequiredFeatures...)}
	before := m.LegacyCapabilities()
	if !before.ExecutionReady || before.MetaMCP || before.MetaDelegation || before.MCPOAuth || before.BuiltinEnabled {
		t.Fatal(before)
	}
	if e := m.MountLegacyMarketMCP(mux, opts); e != nil {
		t.Fatal(e)
	}
	after := m.LegacyCapabilities()
	if !after.ExecutionReady || !after.MetaMCP || !after.MetaDelegation || after.MCPOAuth || after.BuiltinEnabled {
		t.Fatal(after)
	}
	if m.MountLegacyMarketMCP(mux, opts) != ErrConflict {
		t.Fatal("duplicate mount accepted")
	}
	a.ack.Features = append(a.ack.Features, "actors.oauth", "oauth.enabled")
	if !m.LegacyCapabilities().MCPOAuth {
		t.Fatal("actual OAuth ACK not exposed")
	}
	b := &readinessBuiltin{BuiltinDispatcher: &BuiltinDispatcher{}}
	m.builtinRuntime = b
	if m.LegacyCapabilities().BuiltinEnabled {
		t.Fatal("nonnull runtime became ready without purpose ACK")
	}
	b.ready = true
	if !m.LegacyCapabilities().BuiltinEnabled {
		t.Fatal("purpose ACK ignored")
	}
	a.ack = LegacyCapabilities{}
	if m.LegacyCapabilities() != (LegacyRuntimeCapabilities{}) {
		t.Fatal("old mount retained failed readiness")
	}
}
