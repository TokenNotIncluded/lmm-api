// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package rustbridge

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"errors"
	"net"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"

	market "github.com/TokenNotIncluded/lmm-api/services/extensions/internal/modules/toolmarket"
)

// Synthetic UDS transfer test; independent real CorePG verification is separate.
func TestLegacyAccessV2UDSClosedContract(t *testing.T) {
	root := os.Getenv("LMM_TEST_SOCKET_ROOT")
	if root == "" {
		root = t.TempDir()
	}
	socket := filepath.Join(root, "s")
	listener, e := net.Listen("unix", socket)
	if e != nil {
		t.Fatal(e)
	}
	host := strings.Repeat("h", 43)
	secret := "lmm_market_" + base64.RawURLEncoding.EncodeToString(make([]byte, 32))
	p := market.Principal{User: "17", Account: "17"}
	mu := sync.Mutex{}
	seen := []string{}
	bad := false
	server := &http.Server{Handler: http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		mu.Lock()
		defer mu.Unlock()
		seen = append(seen, r.URL.Path)
		credential := "session"
		if strings.HasSuffix(r.URL.Path, "token-validate") {
			credential = secret
		}
		if r.Method != http.MethodPost || r.Header.Get("Authorization") != "Bearer "+host || r.Header.Get("X-Lmm-Protocol") != "2" || r.Header.Get("X-Lmm-User-Credential") != credential {
			t.Error("v2 UDS lost host/protocol/original credential")
			w.WriteHeader(401)
			return
		}
		action := strings.TrimPrefix(r.URL.Path, "/internal/tool-market/v2/access/")
		var in map[string]any
		if json.NewDecoder(r.Body).Decode(&in) != nil {
			w.WriteHeader(400)
			return
		}
		if action != "token-validate" {
			owner, ok := in["owner"].(map[string]any)
			if !ok || owner["user"] != "17" || owner["account"] != "17" {
				t.Error("owner DTO lost")
				w.WriteHeader(403)
				return
			}
		} else if in["account"] != "17" {
			t.Error("validate account lost")
			w.WriteHeader(403)
			return
		}
		if bad {
			json.NewEncoder(w).Encode(map[string]any{"unknown": true})
			return
		}
		switch action {
		case "consent-create":
			if in["tool_id"] != "tool" || in["max_total_quota"] != float64(100) {
				t.Error("consent intent mutated")
			}
			in["id"] = "consent"
			in["quote_id"] = "quote"
			in["frozen_price_quota"] = 10
			in["rules_hash"] = strings.Repeat("a", 64)
			in["created_at"] = 99
			in["revoked_at"] = 0
			in["reserved_quota"] = 3
			in["spent_quota"] = 7
			in["reserved_calls"] = 1
			in["successful_calls"] = 1
		case "consent-revoke", "token-revoke":
			if in["id"] != "target" {
				t.Error("revocation target lost")
			}
			in["revoked_at"] = 100
		case "budget-set":
			if in["scope"] != "account" || in["scope_id"] != "" {
				t.Error("budget scope lost")
			}
			in["reserved_quota"] = 3
			in["spent_quota"] = 7
		case "token-create":
			in["id"] = "token"
			in["auth_version"] = 3
			in["created_at"] = 99
			in["revoked_at"] = 0
			in["token"] = secret
			in["replayed"] = false
		case "token-validate":
			in = map[string]any{"owner": p, "operation_id": "", "client_id": "Client", "can_invoke": true, "can_manage": false, "expires_at": 200, "id": "token", "auth_version": 3, "created_at": 99, "revoked_at": 0, "replayed": false}
		case "client-disconnect":
			in["tokens_revoked"] = 1
			in["grants_revoked"] = 2
			in["revoked_at"] = 100
		case "consent-list", "budget-list", "token-list":
			in = map[string]any{"owner": p, "rows": []any{}}
		default:
			t.Errorf("unexpected v2 endpoint %s", r.URL.Path)
			w.WriteHeader(404)
			return
		}
		json.NewEncoder(w).Encode(in)
	})}
	done := make(chan error, 1)
	go func() { done <- server.Serve(listener) }()
	t.Cleanup(func() { server.Close(); <-done })
	a, e := New(socket, []byte(host))
	if e != nil {
		t.Fatal(e)
	}
	defer a.Close()
	ctx := context.Background()
	op := market.CoreLegacyOperation{Owner: p, OperationID: "op"}
	consent, e := a.CreateLegacyConsent(ctx, "session", market.CoreLegacyConsentRequest{CoreLegacyOperation: op, ClientID: "Client", ToolID: "tool", VersionID: "version", MaxPriceQuota: 10, MaxTotalQuota: 100, MaxCalls: 10, ExpiresAt: 200})
	if e != nil || consent.ReservedQuota != 3 || consent.RulesHash != strings.Repeat("a", 64) {
		t.Fatalf("%+v %v", consent, e)
	}
	id := market.CoreLegacyIDRequest{CoreLegacyOperation: op, ID: "target"}
	if _, e = a.RevokeLegacyConsent(ctx, "session", id); e != nil {
		t.Fatal(e)
	}
	budget := market.CoreLegacyBudgetRequest{CoreLegacyOperation: op, Scope: "account", ScopeID: "", LimitQuota: 100}
	b, e := a.SetLegacyBudget(ctx, "session", budget)
	if e != nil || b.SpentQuota != 7 {
		t.Fatalf("%+v %v", b, e)
	}
	tok, e := a.CreateLegacyToken(ctx, "session", market.CoreLegacyTokenRequest{CoreLegacyOperation: op, ClientID: "Client", CanInvoke: true, ExpiresAt: 200})
	if e != nil || tok.Raw != secret {
		t.Fatalf("%+v %v", tok, e)
	}
	if _, e = a.RevokeLegacyToken(ctx, "session", id); e != nil {
		t.Fatal(e)
	}
	tok, e = a.ValidateLegacyToken(ctx, secret, market.CoreLegacyValidateRequest{Account: "17"})
	if e != nil || tok.Owner != p || tok.Raw != "" || tok.CanManage {
		t.Fatalf("%+v %v", tok, e)
	}
	if _, e = a.DisconnectLegacyClient(ctx, "session", market.CoreLegacyClientRequest{CoreLegacyOperation: op, ClientID: "Client"}); e != nil {
		t.Fatal(e)
	}
	query := market.CoreLegacyListRequest{Owner: p, Offset: 0, Limit: 20}
	if _, e = a.ListLegacyConsents(ctx, "session", query); e != nil {
		t.Fatal(e)
	}
	if _, e = a.ListLegacyBudgets(ctx, "session", query); e != nil {
		t.Fatal(e)
	}
	if _, e = a.ListLegacyTokens(ctx, "session", query); e != nil {
		t.Fatal(e)
	}
	mu.Lock()
	if len(seen) != 10 {
		t.Errorf("10 endpoints required, got %d", len(seen))
	}
	bad = true
	mu.Unlock()
	if _, e = a.SetLegacyBudget(ctx, "session", budget); !errors.Is(e, market.ErrUnavailable) {
		t.Fatalf("unknown response fields accepted: %v", e)
	}
}
