// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package rustbridge

import (
	"context"
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

// Synthetic Core transport contracts only. Real CorePG/ledger acceptance is a
// separate integration run; these tests verify UDS/header/DTO/readback behavior.
func TestUds(t *testing.T) {
	path := filepath.Join(t.TempDir(), "core.sock")
	listener, err := net.Listen("unix", path)
	if err != nil {
		t.Fatal(err)
	}
	var mu sync.Mutex
	state := "reserved"
	paths := []string{}
	token := strings.Repeat("h", 43)
	call := market.Charge{ID: strings.Repeat("a", 64), IdempotencyKey: "original", Installation: "install", Release: "release", Tool: "echo", Quote: strings.Repeat("b", 64), Currency: "USD", Amount: 200, Grant: "grant", Handle: strings.Repeat("c", 64), ArgumentsHash: strings.Repeat("d", 64), Approval: strings.Repeat("e", 64), Generation: 1}
	server := &http.Server{Handler: http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		mu.Lock()
		defer mu.Unlock()
		paths = append(paths, r.URL.Path)
		if r.Header.Get("Authorization") != "Bearer "+token || r.Header.Get("X-Lmm-Protocol") != "1" {
			t.Error("host/protocol scope lost")
			w.WriteHeader(401)
			return
		}
		action := strings.TrimPrefix(r.URL.Path, "/internal/tool-market/v1/")
		if action == "reserve" || action == "dispatch" || action == "authorize" {
			if r.Header.Get("X-Lmm-User-Credential") != "session" {
				t.Error("missing original user credential")
				w.WriteHeader(401)
				return
			}
		} else if r.Header.Get("X-Lmm-User-Credential") != "" {
			t.Error("terminal/reconcile unexpectedly uses current user")
		}
		switch action {
		case "authorize":
			json.NewEncoder(w).Encode(market.Principal{User: "17", Account: "17"})
			return
		case "reserve", "reconcile":
			var got market.Charge
			if err := json.NewDecoder(r.Body).Decode(&got); err != nil || got != call {
				t.Error("immutable charge changed")
				w.WriteHeader(409)
				return
			}
		case "dispatch":
			state = "dispatched"
		case "commit":
			var in struct {
				OperationID string `json:"operation_id"`
				Reservation string `json:"reservation"`
				ResultHash  string `json:"result_hash"`
			}
			if json.NewDecoder(r.Body).Decode(&in) != nil || in.ResultHash != strings.Repeat("f", 64) || in.Reservation != "tool-market:journal:8" {
				t.Error("commit lost original hold/result hash")
				w.WriteHeader(409)
				return
			}
			state = "committed"
		default:
			w.WriteHeader(404)
			return
		}
		json.NewEncoder(w).Encode(receipt{OperationID: call.ID, Reservation: "tool-market:journal:8", State: state, Credits: 100, JournalID: 8})
	})}
	done := make(chan error, 1)
	go func() { done <- server.Serve(listener) }()
	t.Cleanup(func() { server.Close(); <-done; os.Remove(path) })
	adapter, err := New(path, []byte(token))
	if err != nil {
		t.Fatal(err)
	}
	defer adapter.Close()
	ctx := context.Background()
	p := market.Principal{User: "17", Account: "17"}
	got, err := adapter.Authorize(ctx, "session", "17", "toolmarket.execute")
	if err != nil || got != p {
		t.Fatal(got, err)
	}
	if err := adapter.Reserve(ctx, "session", p, call); err != nil {
		t.Fatal(err)
	}
	if err := adapter.MarkDispatched(ctx, "session", p, call); err != nil {
		t.Fatal(err)
	}
	if err := adapter.CommitResult(ctx, "expired-session", p, call, strings.Repeat("f", 64)); err != nil {
		t.Fatal(err)
	}
	if err := adapter.ReleaseUndispatched(ctx, "session", p, call); !errors.Is(err, market.ErrPending) {
		t.Fatal("released dispatched/committed effect", err)
	}
	if err := adapter.Commit(ctx, "session", p, call); !errors.Is(err, market.ErrInvalid) {
		t.Fatal("committed without result evidence", err)
	}
	mu.Lock()
	defer mu.Unlock()
	if len(paths) != 7 {
		t.Fatal("unexpected external requests", paths)
	}
}
func TestReceipt(t *testing.T) {
	c := market.Charge{ID: strings.Repeat("a", 64), Currency: "USD", Amount: 200}
	for _, r := range []*receipt{nil, {OperationID: "wrong", Reservation: "tool-market:journal:1", State: "reserved", Credits: 100, JournalID: 1}, {OperationID: c.ID, Reservation: "tool-market:journal:1", State: "reserved", Credits: 200, JournalID: 1}, {OperationID: c.ID, Reservation: "other", State: "reserved", Credits: 100, JournalID: 1}, {OperationID: c.ID, Reservation: "tool-market:journal:1", State: "unknown", Credits: 100, JournalID: 1}} {
		if err := checkReceipt(r, c); !errors.Is(err, market.ErrPending) {
			t.Fatal("unbound receipt accepted", r, err)
		}
	}
	c.Amount = 0
	if err := checkReceipt(&receipt{OperationID: c.ID, Reservation: "tool-market:free:" + c.ID, State: "reserved"}, c); err != nil {
		t.Fatal(err)
	}
	c.Amount = 1
	if err := checkReceipt(&receipt{OperationID: c.ID, Reservation: "tool-market:free:" + c.ID, State: "reserved"}, c); !errors.Is(err, market.ErrPending) {
		t.Fatal("half-credit silently rounded", err)
	}
}
