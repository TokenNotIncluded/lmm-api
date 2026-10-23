// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package rustbridge

import (
	"context"
	"encoding/json"
	"errors"
	market "github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/toolmarket"
	"net"
	"net/http"
	"path/filepath"
	"strings"
	"sync"
	"testing"
)

func TestLegacyMarketOAuthHostOnlyUDS(t *testing.T) {
	socket := filepath.Join(t.TempDir(), "o")
	listener, e := net.Listen("unix", socket)
	if e != nil {
		t.Fatal(e)
	}
	host := strings.Repeat("h", 43)
	var mu sync.Mutex
	bad := false
	server := &http.Server{Handler: http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		mu.Lock()
		defer mu.Unlock()
		if r.Method != "POST" || r.URL.Path != "/internal/tool-market/v1/oauth-authorize" || r.Header.Get("Authorization") != "Bearer "+host || r.Header.Get("X-Lmm-Protocol") != "1" || r.Header.Get("X-Lmm-User-Credential") != "" {
			t.Error("OAuth wire lost host/purpose boundary")
			w.WriteHeader(403)
			return
		}
		var in market.LegacyMarketOAuthRequest
		decoder := json.NewDecoder(r.Body)
		decoder.DisallowUnknownFields()
		if decoder.Decode(&in) != nil || in.Token != "lmm_at_private" || len(in.RequiredScopes) != 2 || in.RequiredScopes[0] != "market:discover" || in.RequiredScopes[1] != "market:invoke" {
			t.Error("bad OAuth introspection DTO")
			w.WriteHeader(400)
			return
		}
		if bad {
			w.Write([]byte(`{"user_id":"1","account_id":"1","client_id":"oauth:client","credential_kind":"oauth","credential_id":"family","auth_version":0,"expires_at_unix":999,"can_manage":false,"issuer":"https://example.test/api/oauth2/market","resource":"https://example.test/mcp/market"}`))
			return
		}
		json.NewEncoder(w).Encode(market.LegacyMarketPrincipal{UserID: "1", AccountID: "1", ClientID: "oauth:client", CredentialKind: "oauth", CredentialID: "family", ExpiresAt: 999, Issuer: "https://example.test/api/oauth2/market", Resource: "https://example.test/mcp/market", CanInvoke: true})
	})}
	done := make(chan error, 1)
	go func() { done <- server.Serve(listener) }()
	t.Cleanup(func() { server.Close(); <-done })
	adapter, e := New(socket, []byte(host))
	if e != nil {
		t.Fatal(e)
	}
	defer adapter.Close()
	in := market.LegacyMarketOAuthRequest{Token: "lmm_at_private", RequiredScopes: []string{"market:discover", "market:invoke"}}
	out, e := adapter.AuthorizeLegacyMarketOAuth(context.Background(), in)
	if e != nil || out.AccountID != "1" || !out.CanInvoke {
		t.Fatal(out, e)
	}
	mu.Lock()
	bad = true
	mu.Unlock()
	if _, e = adapter.AuthorizeLegacyMarketOAuth(context.Background(), in); !errors.Is(e, market.ErrUnavailable) {
		t.Fatal("missing false scope became known false", e)
	}
}
