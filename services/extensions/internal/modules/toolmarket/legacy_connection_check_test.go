// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package toolmarket

import (
	"context"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync/atomic"
	"testing"
	"time"
)

// These exercise the module against real local HTTP exchanges. The existing
// test authority is not a production Core or a real Treg account acceptance.
func tregCheckFixture(t *testing.T, handler http.HandlerFunc) (*Module, *webTestAuthority, *httptest.Server) {
	t.Helper()
	server := httptest.NewServer(handler)
	t.Cleanup(server.Close)
	authority := &webTestAuthority{}
	store := &Store{data: emptyDB()}
	m, err := New(store, authority, Config{WebOnly: true, CallsPerMinute: 600})
	if err != nil {
		t.Fatal(err)
	}
	client := server.Client()
	client.CheckRedirect = func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }
	m.net = &network{client: client, test: true}
	t.Cleanup(client.CloseIdleConnections)
	t.Cleanup(func() { _ = store.Close() })
	return m, authority, server
}

func tregCheckInput(endpoint string) map[string]any {
	return map[string]any{"provider": "treg", "endpoint": endpoint, "authentication": map[string]string{"mode": "bearer", "secret": "fixture-team-key"}}
}

func TestWebTregConnectionCheckAuthenticatedIdentity(t *testing.T) {
	for _, tc := range []struct {
		name, path, body, want string
		status                 int
	}{
		{"team", "/mcp/", `{"email":"private@example.test","org_id":7,"org":"private-team","role":"member"}`, "verified", 200},
		{"catalog", "/mcp/v2", `{"email":"private@example.test","org_id":7,"org":"private-team"}`, "verified", 200},
		{"invalid-key", "/mcp/", `{"detail":"fixture-team-key"}`, "unauthorized", 401},
		{"forbidden", "/mcp/", `{"detail":"private-team"}`, "forbidden", 403},
		{"unavailable", "/mcp/", `{"detail":"private-team"}`, "unavailable", 503},
		{"identity-without-team", "/mcp/", `{"email":"private@example.test"}`, "team_required", 200},
		{"invalid-json", "/mcp/", `{`, "invalid_response", 200},
		{"empty-identity", "/mcp/", `{}`, "invalid_response", 200},
	} {
		t.Run(tc.name, func(t *testing.T) {
			var requests atomic.Int32
			m, _, server := tregCheckFixture(t, func(w http.ResponseWriter, r *http.Request) {
				requests.Add(1)
				if r.Method != http.MethodGet || r.URL.Path != "/auth/me" || r.URL.RawQuery != "" || r.Header.Get("X-Treg-Token") != "fixture-team-key" || r.Header.Get("Authorization") != "" || r.Header.Get("Cookie") != "" {
					t.Error("identity read used an unexpected path, method or credential boundary")
				}
				w.Header().Set("Content-Type", "application/json")
				w.WriteHeader(tc.status)
				_, _ = w.Write([]byte(tc.body))
			})
			value, err := webRequest(t, m, "1", "connection.check", "", tregCheckInput(server.URL+tc.path))
			if err != nil {
				t.Fatal(err)
			}
			out := value.(WebConnectionCheck)
			wantAuth, wantSurface := "not_verified", "team"
			if tc.want == "verified" {
				wantAuth = "verified"
			}
			if tc.path == "/mcp/v2" {
				wantSurface = "catalog"
			}
			if out.Provider != "treg" || out.Surface != wantSurface || out.Authentication != wantAuth || out.Status != tc.want || out.UpstreamStatus != tc.status || requests.Load() != 1 {
				t.Fatal(out, requests.Load())
			}
			raw, _ := json.Marshal(out)
			for _, private := range []string{"fixture-team-key", "private@example.test", "private-team"} {
				if strings.Contains(string(raw), private) {
					t.Fatal("identity or credential material leaked")
				}
			}
			if err := m.store.view(func(d database) error {
				if d.Web != nil {
					t.Fatal("connection check persisted marketplace records")
				}
				return nil
			}); err != nil {
				t.Fatal(err)
			}
			config, err := m.PublicWebConfig(context.Background())
			if err != nil || !config["capabilities"].(map[string]bool)["treg_connection_check"] {
				t.Fatal("implemented check was not advertised", err)
			}
		})
	}
}

func TestWebTregIdentityAndToolDiscoveryStaySeparate(t *testing.T) {
	var authenticated, empty atomic.Bool
	var toolCalls atomic.Int32
	m, _, server := tregCheckFixture(t, func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		if r.URL.Path == "/auth/me" {
			if !authenticated.Load() {
				w.WriteHeader(http.StatusUnauthorized)
				return
			}
			_, _ = w.Write([]byte(`{"email":"fixture@example.test","org_id":7,"org":"fixture"}`))
			return
		}
		var request struct {
			ID     json.RawMessage `json:"id"`
			Method string          `json:"method"`
		}
		if r.URL.Path != "/mcp/" || json.NewDecoder(r.Body).Decode(&request) != nil {
			w.WriteHeader(http.StatusBadRequest)
			return
		}
		// At the pinned Treg source a missing bearer is challenged, while a
		// non-OAuth bearer can reach static schemas before its key is validated.
		if request.Method != "notifications/initialized" && r.Header.Get("Authorization") == "" {
			w.WriteHeader(http.StatusUnauthorized)
			return
		}
		var result any
		switch request.Method {
		case "initialize":
			result = map[string]any{"protocolVersion": ProtocolVersion, "capabilities": map[string]any{"tools": map[string]any{}}}
		case "notifications/initialized":
			w.WriteHeader(http.StatusAccepted)
			return
		case "tools/list":
			tools := []map[string]any{}
			if !empty.Load() {
				for _, name := range []string{"catalog_search", "image_generate", "custom.team_tool"} {
					tools = append(tools, map[string]any{"name": name, "inputSchema": map[string]any{"type": "object", "properties": map[string]any{}}})
				}
			}
			result = map[string]any{"tools": tools}
		case "tools/call":
			toolCalls.Add(1)
			w.WriteHeader(http.StatusForbidden)
			return
		default:
			w.WriteHeader(http.StatusBadRequest)
			return
		}
		_ = json.NewEncoder(w).Encode(map[string]any{"jsonrpc": "2.0", "id": request.ID, "result": result})
	})
	endpoint := server.URL + "/mcp/"
	if _, err := webRequest(t, m, "1", "inspect", "", map[string]any{"endpoint": endpoint}); !errors.Is(err, ErrRemoteUnauthorized) {
		t.Fatal("missing credential was not challenged", err)
	}
	inspect := map[string]any{"endpoint": endpoint, "authentication": map[string]string{"mode": "bearer", "secret": "fixture-team-key"}}
	value, err := webRequest(t, m, "1", "inspect", "", inspect)
	if err != nil || len(value.([]WebToolInput)) != 3 {
		t.Fatal("full discovery must keep media and custom tools", value, err)
	}
	value, err = webRequest(t, m, "1", "connection.check", "", tregCheckInput(endpoint))
	if err != nil || value.(WebConnectionCheck).Status != "unauthorized" {
		t.Fatal("static discovery was mistaken for authentication", value, err)
	}
	authenticated.Store(true)
	empty.Store(true)
	value, err = webRequest(t, m, "1", "connection.check", "", tregCheckInput(endpoint))
	if err != nil || value.(WebConnectionCheck).Authentication != "verified" {
		t.Fatal(value, err)
	}
	value, err = webRequest(t, m, "1", "inspect", "", inspect)
	if err != nil || value.([]WebToolInput) == nil || len(value.([]WebToolInput)) != 0 || toolCalls.Load() != 0 {
		t.Fatal("empty catalogue must be explicit and checking must not call tools", value, err, toolCalls.Load())
	}
}

func TestWebTregCheckReusesBoundCredentialsAndLiveAuthority(t *testing.T) {
	var requests atomic.Int32
	m, authority, server := tregCheckFixture(t, func(w http.ResponseWriter, r *http.Request) {
		requests.Add(1)
		if r.Header.Get("X-Treg-Token") != "fixture-team-key" {
			t.Error("stored credential was not used")
		}
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"email":"fixture@example.test","org_id":7,"org":"fixture"}`))
	})
	endpoint := server.URL + "/mcp/"
	saved := webSaved(t, m, "1", webInput(endpoint, "private"))
	if _, err := webRequest(t, m, "1", "credential.configure", saved.ID, WebCredentialInput{VersionID: saved.DraftVersionID, Mode: "bearer", Secret: "fixture-team-key"}); err != nil {
		t.Fatal(err)
	}
	reference := map[string]any{"provider": "treg", "endpoint": endpoint, "service_id": saved.ID, "version_id": saved.DraftVersionID}
	value, err := webRequest(t, m, "1", "connection.check", "", reference)
	if err != nil || value.(WebConnectionCheck).Authentication != "verified" {
		t.Fatal(value, err)
	}
	if _, err := webRequest(t, m, "2", "connection.check", "", reference); !errors.Is(err, ErrMissing) {
		t.Fatal("cross-owner credential lookup", err)
	}
	reference["endpoint"] = server.URL + "/mcp/v2/"
	if _, err := webRequest(t, m, "1", "connection.check", "", reference); !errors.Is(err, ErrConflict) {
		t.Fatal("credential endpoint binding was lost", err)
	}
	reference["endpoint"] = endpoint
	reference["authentication"] = map[string]string{"mode": "bearer", "secret": "fixture-team-key"}
	if _, err := webRequest(t, m, "1", "connection.check", "", reference); !errors.Is(err, ErrInvalid) {
		t.Fatal("ambiguous credential sources accepted", err)
	}
	input := tregCheckInput(endpoint)
	input["authentication"] = map[string]string{"mode": "bearer", "secret": "lmms_private-lmm-session"}
	if _, err := webRequest(t, m, "1", "connection.check", "", input); !errors.Is(err, ErrInvalid) || requests.Load() != 1 {
		t.Fatal("invalid credential reached upstream", err, requests.Load())
	}
	authority.mu.Lock()
	revokeAt := authority.calls + 2
	authority.hook = func(call int) {
		if call == revokeAt {
			authority.mu.Lock()
			authority.revoked = true
			authority.mu.Unlock()
		}
	}
	authority.mu.Unlock()
	if _, err := webRequest(t, m, "1", "connection.check", "", tregCheckInput(endpoint)); !errors.Is(err, ErrAuth) || requests.Load() != 2 {
		t.Fatal("revoked LMM authority delivered an upstream result", err, requests.Load())
	}
}

func TestWebTregCheckNetworkFailuresAndRedirects(t *testing.T) {
	for _, stage := range []string{"headers", "body"} {
		t.Run("timeout-"+stage, func(t *testing.T) {
			m, _, server := tregCheckFixture(t, func(w http.ResponseWriter, r *http.Request) {
				if stage == "body" {
					w.Header().Set("Content-Type", "application/json")
					w.WriteHeader(http.StatusOK)
					w.(http.Flusher).Flush()
				}
				<-r.Context().Done()
			})
			m.timeout = 30 * time.Millisecond
			value, err := webRequest(t, m, "1", "connection.check", "", tregCheckInput(server.URL+"/mcp/"))
			if err != nil || value.(WebConnectionCheck).Status != "timeout" || value.(WebConnectionCheck).Authentication != "not_verified" {
				t.Fatal(value, err)
			}
		})
	}
	t.Run("unreachable", func(t *testing.T) {
		m, _, server := tregCheckFixture(t, func(http.ResponseWriter, *http.Request) {})
		server.Close()
		value, err := webRequest(t, m, "1", "connection.check", "", tregCheckInput(server.URL+"/mcp/"))
		if err != nil || value.(WebConnectionCheck).Status != "network_error" {
			t.Fatal(value, err)
		}
	})
	t.Run("redirect", func(t *testing.T) {
		var followed atomic.Int32
		target := httptest.NewServer(http.HandlerFunc(func(http.ResponseWriter, *http.Request) { followed.Add(1) }))
		defer target.Close()
		m, _, server := tregCheckFixture(t, func(w http.ResponseWriter, r *http.Request) {
			http.Redirect(w, r, target.URL+"/auth/me", http.StatusFound)
		})
		value, err := webRequest(t, m, "1", "connection.check", "", tregCheckInput(server.URL+"/mcp/"))
		if err != nil || value.(WebConnectionCheck).Status != "unavailable" || value.(WebConnectionCheck).UpstreamStatus != 302 || followed.Load() != 0 {
			t.Fatal("redirect forwarded a credential", value, err, followed.Load())
		}
		m.net.test = false
		if _, err := webRequest(t, m, "1", "connection.check", "", tregCheckInput("https://127.0.0.1/mcp/")); !errors.Is(err, ErrDenied) {
			t.Fatal("private destination accepted", err)
		}
	})
}

func TestWebDiscoveryKeepsUpstreamAuthSeparateFromLMM(t *testing.T) {
	for _, tc := range []struct {
		status int
		want   error
	}{{401, ErrRemoteUnauthorized}, {403, ErrRemoteForbidden}} {
		t.Run(http.StatusText(tc.status), func(t *testing.T) {
			m, _, server := tregCheckFixture(t, func(w http.ResponseWriter, r *http.Request) { w.WriteHeader(tc.status) })
			_, err := webRequest(t, m, "1", "inspect", "", map[string]any{"endpoint": server.URL + "/mcp/"})
			if !errors.Is(err, tc.want) || errors.Is(err, ErrAuth) || !errors.Is(err, ErrUpstream) {
				t.Fatal("upstream authentication became local session failure", err)
			}
		})
	}
	t.Run("timeout", func(t *testing.T) {
		m, _, server := tregCheckFixture(t, func(w http.ResponseWriter, r *http.Request) {
			_, _ = io.Copy(io.Discard, r.Body)
			<-r.Context().Done()
		})
		m.timeout = 30 * time.Millisecond
		_, err := webRequest(t, m, "1", "inspect", "", map[string]any{"endpoint": server.URL + "/mcp/"})
		if !errors.Is(err, ErrRemoteTimeout) || !errors.Is(err, ErrUpstream) {
			t.Fatal(err)
		}
	})
}
