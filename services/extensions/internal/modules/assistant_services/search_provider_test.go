// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later
package assistant_services

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"net"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync/atomic"
	"testing"
	"time"
)

func TestSearchProviderMainWireProtocols(t *testing.T) {
	for _, provider := range []string{"generic_http", "exa", "tavily", "brave"} {
		t.Run(provider, func(t *testing.T) {
			var calls atomic.Int32
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				calls.Add(1)
				if r.Header.Get("Accept") != "application/json" {
					t.Error("missing Accept")
				}
				switch provider {
				case "exa":
					if r.Method != "POST" || r.Header.Get("X-Api-Key") != "synthetic-secret" {
						t.Error("bad Exa auth/method")
					}
					var body map[string]any
					if json.NewDecoder(r.Body).Decode(&body) != nil || body["query"] != "bounded query" || body["contents"].(map[string]any)["highlights"] != true {
						t.Error(body)
					}
				case "tavily":
					if r.Method != "POST" || r.Header.Get("Authorization") != "Bearer synthetic-secret" {
						t.Error("bad Tavily auth/method")
					}
					var body map[string]any
					if json.NewDecoder(r.Body).Decode(&body) != nil || body["query"] != "bounded query" {
						t.Error(body)
					}
				case "brave":
					if r.Method != "GET" || r.URL.Query().Get("q") != "bounded query" || r.Header.Get("X-Subscription-Token") != "synthetic-secret" {
						t.Error("bad Brave request")
					}
				case "generic_http":
					if r.Method != "GET" || r.URL.Query().Get("q") != "bounded query" || r.Header.Get("Authorization") != "Bearer synthetic-secret" || r.Header.Get("X-Api-Key") != "synthetic-secret" {
						t.Error("bad generic request")
					}
				}
				fmt.Fprint(w, `{"results":[{"text":"synthetic-secret","nested":{"synthetic-secret":"ok"}}]}`)
			}))
			defer server.Close()
			p, err := NewSearchProvider(SearchConfig{Provider: provider, Endpoint: server.URL, APIKey: "synthetic-secret", LocalHTTP: true})
			if err != nil {
				t.Fatal(err)
			}
			raw, err := p.Search(context.Background(), "bounded query")
			if err != nil || strings.Contains(string(raw), "synthetic-secret") {
				t.Fatal(string(raw), err)
			}
			var result map[string]any
			if json.Unmarshal(raw, &result) != nil || result["ok"] != true || result["source_is_untrusted"] != true || calls.Load() != 1 {
				t.Fatal(string(raw), calls.Load())
			}
		})
	}
}

func TestSearchUnknownNeverRetriesOrFollowsRedirect(t *testing.T) {
	for _, mode := range []string{"redirect", "non2xx", "oversized", "disconnect", "timeout"} {
		t.Run(mode, func(t *testing.T) {
			var calls, target atomic.Int32
			destination := httptest.NewServer(http.HandlerFunc(func(http.ResponseWriter, *http.Request) { target.Add(1) }))
			defer destination.Close()
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				calls.Add(1)
				switch mode {
				case "redirect":
					http.Redirect(w, r, destination.URL, 302)
				case "non2xx":
					w.WriteHeader(429)
					fmt.Fprint(w, "synthetic-secret")
				case "oversized":
					fmt.Fprint(w, strings.Repeat("x", 65537))
				case "timeout":
					time.Sleep(50 * time.Millisecond)
				case "disconnect":
					conn, _, _ := w.(http.Hijacker).Hijack()
					conn.Close()
				}
			}))
			defer server.Close()
			p, err := NewSearchProvider(SearchConfig{Provider: "generic_http", Endpoint: server.URL, APIKey: "synthetic-secret", LocalHTTP: true, Timeout: 15 * time.Millisecond})
			if err != nil {
				t.Fatal(err)
			}
			raw, err := p.Search(context.Background(), "test query")
			var failure *SearchError
			if !errors.As(err, &failure) || !failure.DispatchUnknown() || calls.Load() != 1 || target.Load() != 0 || strings.Contains(string(raw), "synthetic-secret") {
				t.Fatal(string(raw), err, calls.Load(), target.Load())
			}
		})
	}
}

func TestSearchValidationBeforeDispatch(t *testing.T) {
	for _, config := range []SearchConfig{{Provider: "unregistered_provider", Endpoint: "https://example.com"}, {Provider: "exa"}, {Provider: "generic_http", Endpoint: "http://127.0.0.1"}, {Provider: "generic_http", Endpoint: "https://169.254.169.254"}, {Provider: "generic_http", Endpoint: "http://localhost", LocalHTTP: true}, {Provider: "generic_http", Endpoint: "http://example.com", LocalHTTP: true}} {
		if _, err := NewSearchProvider(config); err == nil {
			t.Fatal(config.Provider, config.Endpoint)
		}
	}
	var calls atomic.Int32
	server := httptest.NewServer(http.HandlerFunc(func(http.ResponseWriter, *http.Request) { calls.Add(1) }))
	defer server.Close()
	p, err := NewSearchProvider(SearchConfig{Provider: "generic_http", Endpoint: server.URL, LocalHTTP: true})
	if err != nil {
		t.Fatal(err)
	}
	for _, query := range []string{"x", strings.Repeat("x", 2001), "\x00query"} {
		_, err := p.Search(context.Background(), query)
		var failure *SearchError
		if !errors.As(err, &failure) || failure.Unknown {
			t.Fatal(err)
		}
	}
	if calls.Load() != 0 {
		t.Fatal(calls.Load())
	}
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	endpoint := "http://" + listener.Addr().String()
	listener.Close()
	p, err = NewSearchProvider(SearchConfig{Provider: "generic_http", Endpoint: endpoint, LocalHTTP: true})
	if err != nil {
		t.Fatal(err)
	}
	_, err = p.Search(context.Background(), "test query")
	var failure *SearchError
	if !errors.As(err, &failure) || failure.Unknown {
		t.Fatal(err)
	}
}
