// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package rustbridge

import (
	"context"
	"encoding/json"
	"errors"
	market "github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/toolmarket"
	"math"
	"net"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
)

func TestLegacyQuoteV2UDSAuthorityContract(t *testing.T) {
	root := os.Getenv("LMM_TEST_SOCKET_ROOT")
	if root == "" {
		root = t.TempDir()
	}
	socket := filepath.Join(root, "q")
	listener, e := net.Listen("unix", socket)
	if e != nil {
		t.Fatal(e)
	}
	host := strings.Repeat("h", 43)
	var mu sync.Mutex
	bad := false
	calls := 0
	q := market.LegacyCreditQuote{ID: strings.Repeat("a", 64), ServiceID: "service", ToolID: "tool", VersionID: "version", RemoteDigest: strings.Repeat("b", 64), PricingDigest: strings.Repeat("c", 64), Price: market.LegacyQuotePrice{Mode: "fixed", PriceQuota: 0}, Expires: math.MaxInt64}
	server := &http.Server{Handler: http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		mu.Lock()
		defer mu.Unlock()
		calls++
		credential := "publisher-session"
		if r.URL.Path == "/internal/tool-market/v2/quotes/approve" {
			credential = "admin-session"
		}
		if r.Method != "POST" || r.Header.Get("Authorization") != "Bearer "+host || r.Header.Get("X-Lmm-Protocol") != "2" || r.Header.Get("X-Lmm-User-Credential") != credential {
			t.Error("quote authority header changed")
			w.WriteHeader(403)
			return
		}
		switch r.URL.Path {
		case "/internal/tool-market/v2/quotes":
			var got market.LegacyCreditQuote
			if e := json.NewDecoder(r.Body).Decode(&got); e != nil || got.ID != q.ID || got.Price.Mode != "fixed" || got.Price.PriceQuota != 0 || got.Expires != math.MaxInt64 {
				t.Error("quote snapshot changed", e)
				w.WriteHeader(400)
				return
			}
			json.NewEncoder(w).Encode(got)
		case "/internal/tool-market/v2/quotes/approve":
			var got market.LegacyQuoteApproval
			if e := json.NewDecoder(r.Body).Decode(&got); e != nil || got.QuoteID != q.ID || got.Visibility != "public" || !got.Approved {
				t.Error("approval intent changed", e)
				w.WriteHeader(400)
				return
			}
			if bad {
				json.NewEncoder(w).Encode(map[string]any{"operation_id": got.OperationID, "quote_id": got.QuoteID, "visibility": got.Visibility, "approved_by": "3", "approved_at": 100})
				return
			}
			json.NewEncoder(w).Encode(market.LegacyQuoteApprovalReceipt{LegacyQuoteApproval: got, ApprovedBy: "3", ApprovedAt: 100})
		default:
			t.Error("unexpected quote path", r.URL.Path)
			w.WriteHeader(404)
		}
	})}
	done := make(chan error, 1)
	go func() { done <- server.Serve(listener) }()
	t.Cleanup(func() { server.Close(); <-done })
	a, e := New(socket, []byte(host))
	if e != nil {
		t.Fatal(e)
	}
	defer a.Close()
	out, e := a.RegisterLegacyQuote(context.Background(), "publisher-session", q)
	if e != nil || out.ID != q.ID || out.Price.Mode != "fixed" || out.Price.PriceQuota != 0 {
		t.Fatal(out, e)
	}
	in := market.LegacyQuoteApproval{OperationID: "approve", QuoteID: q.ID, Visibility: "public", Approved: true}
	approved, e := a.ApproveLegacyQuote(context.Background(), "admin-session", in)
	if e != nil || approved.LegacyQuoteApproval != in || approved.ApprovedBy != "3" {
		t.Fatal(approved, e)
	}
	mu.Lock()
	bad = true
	mu.Unlock()
	if _, e = a.ApproveLegacyQuote(context.Background(), "admin-session", in); !errors.Is(e, market.ErrUnavailable) {
		t.Fatal("missing approved field accepted", e)
	}
	mu.Lock()
	defer mu.Unlock()
	if calls != 3 {
		t.Fatal("unexpected authority calls", calls)
	}
}
