// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package webcompat

import (
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/toolmarket"
	"net/http"
	"net/http/httptest"
	"testing"
)

type marketRuntimeFake struct {
	marketTransportFake
	requests []*http.Request
}

func (f *marketRuntimeFake) ToolMarketRuntimeHandler() http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { f.requests = append(f.requests, r); w.WriteHeader(202) })
}
func TestToolMarketRuntimeUsesVerifiedActorAndPreservesOAuthBinding(t *testing.T) {
	core, f := &authCore{}, &marketRuntimeFake{}
	req := marketHTTP("POST", "/api/tool-market/runtime/meta/search?query=keep", "{}", true)
	req.Header["x-lmm-user-credential"] = []string{"attacker"}
	req.Header["X-LMM-Account-ID"] = []string{"999"}
	req.Header["X-LMM-Host-Token"] = []string{"forged"}
	req.Header.Set("Cookie", "__Host-lmm-mcp-binding=synthetic")
	req.Header.Set("Origin", "https://synthetic.invalid")
	rec := httptest.NewRecorder()
	marketMux(t, core, f).ServeHTTP(rec, req)
	if rec.Code != 202 || len(f.requests) != 1 {
		t.Fatal(rec.Code, len(f.requests))
	}
	got := f.requests[0]
	if got.Header.Get("X-LMM-User-Credential") != webToken || got.Header.Get("X-LMM-Account-ID") != "17" || got.Header.Get("X-LMM-Host-Token") != "" || got.URL.Path != "/meta/search" || got.URL.RawQuery != "query=keep" || got.Header.Get("Cookie") != req.Header.Get("Cookie") || got.Header.Get("Origin") != req.Header.Get("Origin") {
		t.Fatal("runtime actor binding or OAuth transport changed")
	}
	if req.URL.Path != "/api/tool-market/runtime/meta/search" || req.Header["X-LMM-Account-ID"][0] != "999" {
		t.Fatal("mutated caller request")
	}
}
func TestToolMarketRuntimeRejectsMissingSessionAndDeniedActionBeforeReadingBody(t *testing.T) {
	for _, hasToken := range []bool{false, true} {
		core, f := &authCore{}, &marketRuntimeFake{marketTransportFake: marketTransportFake{authErr: toolmarket.ErrDenied}}
		req := marketHTTP("POST", "/api/tool-market/runtime/meta/execute", "", hasToken)
		body := &marketUnreadBody{}
		req.Body = body
		rec := httptest.NewRecorder()
		marketMux(t, core, f).ServeHTTP(rec, req)
		want := 401
		if hasToken {
			want = 403
		}
		if rec.Code != want || len(f.requests) != 0 || body.reads != 0 {
			t.Fatal(rec.Code, len(f.requests), body.reads)
		}
	}
}
