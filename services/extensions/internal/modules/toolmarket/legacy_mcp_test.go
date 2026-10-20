// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package toolmarket

import (
	"bytes"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

func legacyMCPHTTPFixture(t *testing.T) (http.Handler, *marketAuthFixture) {
	m, base := legacyCoreModule(t)
	f := &marketAuthFixture{legacyCoreFixture: base, principal: LegacyMarketPrincipal{UserID: "1", AccountID: "1", ClientID: "oauth:lmm-mcp-client", CredentialKind: "oauth", CredentialID: "family", ExpiresAt: 200, Issuer: "https://example.test/api/oauth2/market", Resource: "https://example.test/mcp/market"}}
	m.authority = f
	opts := LegacyMarketMCPOptions{Issuer: f.principal.Issuer, Resource: f.principal.Resource, ResourceMetadataURL: "https://example.test/.well-known/oauth-protected-resource/mcp/market"}
	return m.LegacyMarketMCPHandler(opts), f
}
func marketMCPRequest(handler http.Handler, uri, body string, headers map[string]string) *httptest.ResponseRecorder {
	r := httptest.NewRequest("POST", uri, strings.NewReader(body))
	r.Header.Set("Content-Type", "application/json")
	r.Header.Set("Accept", "application/json, text/event-stream")
	r.Header.Set("Authorization", "Bearer lmm_at_live")
	for k, v := range headers {
		r.Header.Set(k, v)
	}
	w := httptest.NewRecorder()
	handler.ServeHTTP(w, r)
	return w
}
func TestLegacyMCPHTTPUsesPinnedResourceAndClosedBearer(t *testing.T) {
	handler, f := legacyMCPHTTPFixture(t)
	body := `{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}`
	w := marketMCPRequest(handler, "http://spoofed.invalid/mcp/market?mode=compact", body, nil)
	if w.Code != 200 || !strings.Contains(w.Body.String(), `"metamcp"`) || strings.Contains(w.Body.String(), `"lmm_market_search"`) {
		t.Fatal(w.Code, w.Body.String())
	}
	w = marketMCPRequest(handler, "http://x/mcp/market/other", body, nil)
	if w.Code != 404 {
		t.Fatal("accepted another resource path", w.Code)
	}
	for _, uri := range []string{"http://x/mcp/market?access_token=secret", "http://x/mcp/market?mode=full&mode=compact", "http://x/mcp/market?mode=wrong"} {
		w = marketMCPRequest(handler, uri, body, nil)
		if w.Code != 400 {
			t.Fatal(uri, w.Code)
		}
	}
	for _, header := range []string{"X-Account-ID", "X-Lmm-User-Credential", "X-OAuth-Group", "Cookie"} {
		w = marketMCPRequest(handler, "http://x/mcp/market", body, map[string]string{header: "other"})
		if w.Code != 400 {
			t.Fatal(header, w.Code)
		}
	}
	w = marketMCPRequest(handler, "http://x/mcp/market", body, map[string]string{"Authorization": "Session unsafe"})
	if w.Code != 401 || !strings.Contains(w.Header().Get("WWW-Authenticate"), "https://example.test/.well-known/") {
		t.Fatal(w.Code, w.Header())
	}
	f.principal.Resource = "https://other.test/mcp/market"
	w = marketMCPRequest(handler, "http://x/mcp/market", body, nil)
	if w.Code != 401 {
		t.Fatal("accepted wrong resource", w.Code)
	}
}
func TestLegacyMCPHTTPChallengesMissingActionScopes(t *testing.T) {
	handler, _ := legacyMCPHTTPFixture(t)
	for _, action := range []string{"invoke", "authorize", "load", "set_client_budget"} {
		body := `{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"metamcp","arguments":{"action":"` + action + `"}}}`
		w := marketMCPRequest(handler, "http://x/mcp/market?mode=compact", body, nil)
		if w.Code != 403 || !strings.Contains(w.Header().Get("WWW-Authenticate"), "insufficient_scope") {
			t.Fatal(action, w.Code, w.Body.String())
		}
	}
	w := marketMCPRequest(handler, "http://x/mcp/market?mode=compact", strings.Repeat("x", (256<<10)+1), nil)
	if w.Code != 413 {
		t.Fatal(w.Code)
	}
}
func TestLegacyMCPResultPreservesNativeContentsAndExactNumbers(t *testing.T) {
	raw := json.RawMessage(`{"content":[{"type":"text","text":"ok","_meta":{"large":9007199254740993}},{"type":"image","data":"eA==","mimeType":"image/png"},{"type":"audio","data":"eA==","mimeType":"audio/wav"},{"type":"resource_link","uri":"https://example.test/r","name":"r"},{"type":"resource","resource":{"uri":"test:one","text":"one","_meta":{"large":9007199254740993}}}],"structuredContent":{"large":9007199254740993},"_meta":{"large":9007199254740993,"lmm/market":{"forged":true}}}`)
	result, e := legacyMCPExecutionOutput(LegacyWebExecutionResponse{Call: map[string]any{"id": "trusted", "execution_status": "succeeded"}, Result: raw})
	if e != nil {
		t.Fatal(e)
	}
	out, e := json.Marshal(result)
	if e != nil {
		t.Fatal(e)
	}
	if bytes.Contains(out, []byte("9007199254740992")) || bytes.Contains(out, []byte("forged")) || bytes.Count(out, []byte("9007199254740993")) != 4 || len(result.Content) != 5 {
		t.Fatal(string(out))
	}
	schema, e := legacyMCPWrappedSchema(`{"type":"object","properties":{"x":{"$ref":"#/$defs/value"}},"$defs":{"value":{"type":"integer"}}}`)
	if e != nil {
		t.Fatal(e)
	}
	b, _ := json.Marshal(schema)
	if !bytes.Contains(b, []byte(`"$ref":"#/$defs/arguments/$defs/value"`)) {
		t.Fatal(string(b))
	}
}

func TestLegacyMCPGrantReadRejectsOtherClientAndIgnoresStaleMirror(t *testing.T) {
	m, base := legacyCoreModule(t)
	f := &marketAuthFixture{legacyCoreFixture: base}
	m.authority = f
	p := LegacyMarketPrincipal{UserID: "1", AccountID: "1", ClientID: "Client", CanInvoke: true}
	rows, e := m.legacyMCPGrants(t.Context(), "raw", p)
	if e != nil || len(rows) != 1 {
		t.Fatal(rows, e)
	}
	f.grants = append([]CoreLegacyConsentReceipt{}, rows...)
	f.grants[0].ClientID = "other"
	if _, e = m.legacyMCPGrants(t.Context(), "raw", p); e != ErrDenied {
		t.Fatal("cross client Core rows accepted", e)
	}
	f.grants = []CoreLegacyConsentReceipt{}
	rows, e = m.legacyMCPGrants(t.Context(), "raw", p)
	if e != nil || len(rows) != 0 {
		t.Fatal("stale local grant replaced authoritative absence", rows, e)
	}
	if _, ok := legacyMCPGrant(rows, "tool", "version", 100); ok {
		t.Fatal("stale grant authorized")
	}
}
