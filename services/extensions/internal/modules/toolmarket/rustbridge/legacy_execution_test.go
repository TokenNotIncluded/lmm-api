// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package rustbridge

import (
	"context"
	"encoding/base64"
	"encoding/json"
	market "github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/toolmarket"
	"net"
	"net/http"
	"path/filepath"
	"strings"
	"sync"
	"testing"
)

// This runs the actual Unix transport consumer with a synthetic Core server.
// It is a wire contract check, not Rust/PostgreSQL financial acceptance.
func TestLegacyExecutionUDSContract(t *testing.T) {
	path := filepath.Join(t.TempDir(), "core.sock")
	listener, e := net.Listen("unix", path)
	if e != nil {
		t.Fatal(e)
	}
	token := strings.Repeat("h", 43)
	largeRaw := base64.StdEncoding.EncodeToString([]byte(`{"content":[{"type":"text","text":"` + strings.Repeat("x", 40000) + `"}]}`))
	seen := []string{}
	var mu sync.Mutex
	server := &http.Server{Handler: http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Header.Get("Authorization") != "Bearer "+token || r.Header.Get("X-Lmm-Protocol") != "2" {
			t.Error("host/protocol changed")
			w.WriteHeader(401)
			return
		}
		mu.Lock()
		defer mu.Unlock()
		action := strings.TrimPrefix(r.URL.Path, "/internal/tool-market/v2/calls/")
		seen = append(seen, action)
		var body map[string]json.RawMessage
		if json.NewDecoder(r.Body).Decode(&body) != nil {
			t.Error("invalid JSON")
			w.WriteHeader(400)
			return
		}
		for _, field := range []string{"payer", "seller", "amount", "amount_usd", "fee", "platform"} {
			if _, ok := body[field]; ok {
				t.Error("caller financial authority", field)
			}
		}
		session := r.Header.Get("X-Lmm-User-Credential")
		if action == "reserve" || action == "dispatch" || action == "query" {
			if session != "original-session" {
				t.Error("lost current session")
			}
		} else if session != "" {
			t.Error("host completion used session")
		}
		switch action {
		case "reserve":
			if len(body) != 9 {
				t.Errorf("reserve fields: %v", body)
			}
		case "dispatch", "query":
			if len(body) != 1 || string(body["call_id"]) != `"call"` {
				t.Error("dispatch/query scope", body)
			}
		case "unknown":
			if len(body) != 2 || string(body["dispatch_id"]) != `"dispatch"` {
				t.Error("unknown scope", body)
			}
		case "settle":
			if len(body) != 5 || string(body["usage"]) != "null" || string(body["result_digest"]) != `"result"` {
				t.Error("settle missing result/usage", body)
			}
			var encoded string
			if json.Unmarshal(body["raw_result"], &encoded) != nil || encoded != largeRaw {
				t.Error("raw original result changed or missing")
			}
		case "release":
			var encoded string
			if json.Unmarshal(body["raw_result"], &encoded) != nil || encoded != largeRaw {
				t.Error("raw failure changed or missing")
			}
			if len(body) != 5 || string(body["reason"]) != `"failed"` || string(body["evidence_digest"]) != `"result"` {
				t.Error("failure release", body)
			}
		default:
			t.Error("unexpected route", action)
			w.WriteHeader(404)
			return
		}
		_ = json.NewEncoder(w).Encode(market.LegacyCreditCall{ID: "call", State: "running", DispatchID: "dispatch", Held: 42})
	})}
	done := make(chan error, 1)
	go func() { done <- server.Serve(listener) }()
	t.Cleanup(func() { _ = server.Close(); <-done })
	adapter, e := New(path, []byte(token))
	if e != nil {
		t.Fatal(e)
	}
	defer adapter.Close()
	ctx := context.Background()
	id := market.LegacyCreditID{Owner: market.Principal{User: "1", Account: "1"}, CallID: "call", DispatchID: "dispatch"}
	reserve := market.LegacyCreditReserve{Owner: id.Owner, ClientID: "web", RequestKey: "key", QuoteID: "quote", GrantID: "grant", ToolID: "tool", VersionID: "version", InputDigest: "input", ResolveBy: 1000}
	if _, e = adapter.ReserveLegacyCredits(ctx, "original-session", reserve); e != nil {
		t.Fatal(e)
	}
	if _, e = adapter.DispatchLegacyCredits(ctx, "original-session", id); e != nil {
		t.Fatal(e)
	}
	if _, e = adapter.QueryLegacyCredits(ctx, "original-session", id); e != nil {
		t.Fatal(e)
	}
	if _, e = adapter.UnknownLegacyCredits(ctx, id); e != nil {
		t.Fatal(e)
	}
	if _, e = adapter.FinishLegacyCredits(ctx, market.LegacyCreditFinish{LegacyCreditID: id, Success: true, ResultDigest: "result", RawResult: largeRaw}); e != nil {
		t.Fatal(e)
	}
	if _, e = adapter.FinishLegacyCredits(ctx, market.LegacyCreditFinish{LegacyCreditID: id, Success: false, ResultDigest: "result", RawResult: largeRaw}); e != nil {
		t.Fatal(e)
	}
	mu.Lock()
	defer mu.Unlock()
	if strings.Join(seen, ",") != "reserve,dispatch,query,unknown,settle,release" {
		t.Fatal(seen)
	}
}
