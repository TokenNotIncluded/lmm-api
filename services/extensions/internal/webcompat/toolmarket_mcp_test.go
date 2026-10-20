// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package webcompat

import (
	"github.com/TokenNotIncluded/api.lmm.best/extensions/internal/modules/toolmarket"
	"net/http"
	"net/http/httptest"
	"testing"
)

type marketMCPRouteFixture struct {
	got toolmarket.LegacyMarketMCPOptions
}

func (f *marketMCPRouteFixture) LegacyMarketMCPHandler(opts toolmarket.LegacyMarketMCPOptions) http.Handler {
	f.got = opts
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Header.Get("Authorization") != "Bearer resource-only" {
			w.WriteHeader(401)
			return
		}
		w.WriteHeader(204)
	})
}
func TestLegacyMarketMCPRoutesPreserveResourceHandler(t *testing.T) {
	mux := http.NewServeMux()
	f := &marketMCPRouteFixture{}
	opts := toolmarket.LegacyMarketMCPOptions{Issuer: "https://example.test/api/oauth2/market", Resource: "https://example.test/mcp/market", ResourceMetadataURL: "https://example.test/.well-known/oauth-protected-resource/mcp/market"}
	RegisterToolMarketLegacyMCP(mux, f, opts)
	if f.got != opts {
		t.Fatal("trusted URLs replaced", f.got)
	}
	for _, path := range []string{"/mcp/market", "/mcp/market/"} {
		r := httptest.NewRequest("POST", path, nil)
		r.Header.Set("Authorization", "Bearer resource-only")
		w := httptest.NewRecorder()
		mux.ServeHTTP(w, r)
		if w.Code != 204 {
			t.Fatal(path, w.Code)
		}
	}
}
