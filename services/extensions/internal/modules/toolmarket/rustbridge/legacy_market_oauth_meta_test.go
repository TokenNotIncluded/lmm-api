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

func TestLegacyMarketMetaV2ClosedUDS(t *testing.T) {
	socket := filepath.Join(t.TempDir(), "m")
	l, e := net.Listen("unix", socket)
	if e != nil {
		t.Fatal(e)
	}
	host := strings.Repeat("h", 43)
	var mu sync.Mutex
	bad := false
	owner := market.Principal{User: "1", Account: "1"}
	policy := market.LegacyMarketDelegation{Enabled: true, MaxTotalQuota: 100, ExpiresAt: 200, UpdatedAt: 100}
	grant := market.CoreLegacyConsentReceipt{CoreLegacyConsentRequest: market.CoreLegacyConsentRequest{CoreLegacyOperation: market.CoreLegacyOperation{Owner: owner, OperationID: "op"}, ClientID: "Client", ToolID: "tool", VersionID: "version", MaxPriceQuota: 10, MaxTotalQuota: 100, MaxCalls: 10, ExpiresAt: 200}, ID: "grant", QuoteID: "quote", RulesHash: strings.Repeat("a", 64), FrozenPriceQuota: 10, CreatedAt: 99, ReservedQuota: 10, SpentQuota: 20}
	server := &http.Server{Handler: http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		mu.Lock()
		defer mu.Unlock()
		if r.Method != "POST" || r.Header.Get("Authorization") != "Bearer "+host || r.Header.Get("X-Lmm-Protocol") != "2" || r.Header.Get("X-Lmm-User-Credential") != "original-resource-or-session" {
			t.Error("meta host/raw/protocol boundary changed")
			w.WriteHeader(403)
			return
		}
		var fields map[string]json.RawMessage
		if json.NewDecoder(r.Body).Decode(&fields) != nil {
			t.Error("invalid body")
			w.WriteHeader(400)
			return
		}
		var out any
		switch strings.TrimPrefix(r.URL.Path, "/internal/tool-market/v2/meta/") {
		case "read":
			if len(fields) != 0 {
				t.Error("read accepts identity override")
			}
			out = market.LegacyMarketMetaView{Owner: owner, ClientID: "Client", Delegation: policy}
		case "owner-read", "owner-set":
			out = market.LegacyMarketMetaOwnerView{OperationID: "op", Owner: owner, Kind: "personal", ID: "token", Delegation: policy}
		case "authorize", "tool-budget":
			out = grant
		case "client-budget":
			out = market.CoreLegacyBudgetReceipt{CoreLegacyBudgetRequest: market.CoreLegacyBudgetRequest{CoreLegacyOperation: market.CoreLegacyOperation{Owner: owner, OperationID: "op"}, Scope: "client", ScopeID: "Client", LimitQuota: 100}, ReservedQuota: 10, SpentQuota: 20}
		case "usage":
			if bad {
				w.Write([]byte(`{"owner":{"user":"1","account":"1"},"client_id":"Client","grants":[],"offset":0,"limit":20,"more_grants":false}`))
				return
			}
			out = market.LegacyMarketMetaUsage{Owner: owner, ClientID: "Client", Grants: []market.CoreLegacyConsentReceipt{grant}, Limit: 20}
		case "oauth-clients":
			if _, ok := fields["require_manage"]; !ok {
				t.Error("picker scope policy omitted")
			}
			out = market.LegacyMarketOAuthClients{Owner: owner, Clients: []market.LegacyMarketOAuthClient{{ClientID: "oauth:lmm-mcp-client"}}, Limit: 20}
		case "calls":
			out = market.LegacyMarketMetaCalls{Owner: owner, ClientID: "Client", Calls: []market.LegacyMarketMetaCall{}, Limit: 20}
		default:
			t.Error("unknown meta path", r.URL.Path)
			w.WriteHeader(404)
			return
		}
		json.NewEncoder(w).Encode(out)
	})}
	done := make(chan error, 1)
	go func() { done <- server.Serve(l) }()
	t.Cleanup(func() { server.Close(); <-done })
	a, e := New(socket, []byte(host))
	if e != nil {
		t.Fatal(e)
	}
	defer a.Close()
	ctx := context.Background()
	raw := "original-resource-or-session"
	if _, e = a.ReadLegacyMarketMeta(ctx, raw); e != nil {
		t.Fatal(e)
	}
	if _, e = a.ReadLegacyMarketMetaOwner(ctx, raw, market.LegacyMarketMetaTarget{Kind: "personal", ID: "token"}); e != nil {
		t.Fatal(e)
	}
	if _, e = a.SetLegacyMarketMetaOwner(ctx, raw, market.LegacyMarketMetaSet{OperationID: "op", Kind: "personal", ID: "token", Enabled: true, MaxTotalQuota: 100, ExpiresAt: 200}); e != nil {
		t.Fatal(e)
	}
	if out, e := a.AuthorizeLegacyMarketMeta(ctx, raw, market.LegacyMarketMetaAuthorize{OperationID: "op", ToolID: "tool", VersionID: "version", MaxPriceQuota: 10, MaxTotalQuota: 100, MaxCalls: 10, ExpiresAt: 200}); e != nil || out.SpentQuota != 20 {
		t.Fatal(out, e)
	}
	if _, e = a.TightenLegacyMarketClientBudget(ctx, raw, market.LegacyMarketMetaClientBudget{OperationID: "op", LimitQuota: 100}); e != nil {
		t.Fatal(e)
	}
	if _, e = a.TightenLegacyMarketToolBudget(ctx, raw, market.LegacyMarketMetaToolBudget{OperationID: "op", GrantID: "grant", ToolID: "tool", VersionID: "version", LimitQuota: 100}); e != nil {
		t.Fatal(e)
	}
	if _, e = a.ReadLegacyMarketUsage(ctx, raw, market.LegacyMarketMetaPage{Limit: 20}); e != nil {
		t.Fatal(e)
	}
	if _, e = a.ReadLegacyMarketCalls(ctx, raw, market.LegacyMarketMetaPage{Limit: 20}); e != nil {
		t.Fatal(e)
	}
	if _, e = a.ListLegacyMarketOAuthClients(ctx, raw, market.LegacyMarketOAuthClientsRequest{Limit: 20, RequireManage: true}); e != nil {
		t.Fatal(e)
	}
	if _, e = a.ListLegacyMarketOAuthClients(ctx, raw, market.LegacyMarketOAuthClientsRequest{Limit: 20, RequireManage: false}); e != nil {
		t.Fatal(e)
	}
	mu.Lock()
	bad = true
	mu.Unlock()
	if _, e = a.ReadLegacyMarketUsage(ctx, raw, market.LegacyMarketMetaPage{Limit: 20}); !errors.Is(e, market.ErrUnavailable) {
		t.Fatal("missing nullablebudget silently absent", e)
	}
}
