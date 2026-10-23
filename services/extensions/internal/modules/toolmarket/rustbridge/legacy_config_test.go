// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package rustbridge

import (
	"context"
	"encoding/json"
	"errors"
	market "github.com/TokenNotIncluded/api.lmm.best/extensions/internal/modules/toolmarket"
	"net"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
)

func TestLegacyConfigV2UDSHostReadAndRootUpdate(t *testing.T) {
	root := os.Getenv("LMM_TEST_SOCKET_ROOT")
	if root == "" {
		root = t.TempDir()
	}
	socket := filepath.Join(root, "c")
	listener, e := net.Listen("unix", socket)
	if e != nil {
		t.Fatal(e)
	}
	host := strings.Repeat("h", 43)
	var mu sync.Mutex
	bad := false
	reads, updates := 0, 0
	server := &http.Server{Handler: http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		mu.Lock()
		defer mu.Unlock()
		if r.Method != "POST" || r.Header.Get("Authorization") != "Bearer "+host || r.Header.Get("X-Lmm-Protocol") != "2" {
			t.Error("config host/protocol missing")
			w.WriteHeader(403)
			return
		}
		switch r.URL.Path {
		case "/internal/tool-market/v2/config/read":
			reads++
			if r.Header.Get("X-Lmm-User-Credential") != "" {
				t.Error("public config unexpectedly sends user Session")
				w.WriteHeader(403)
				return
			}
			var body map[string]any
			if json.NewDecoder(r.Body).Decode(&body) != nil || len(body) != 0 {
				t.Error("config read must be empty closed DTO")
				w.WriteHeader(400)
				return
			}
			if bad {
				json.NewEncoder(w).Encode(map[string]any{"fee_bps": 0, "recipient_id": 0, "updated_by": 0, "updated_at": 0, "version": 1})
				return
			}
			json.NewEncoder(w).Encode(market.LegacyMarketConfig{Version: 1})
		case "/internal/tool-market/v2/config/update":
			updates++
			if r.Header.Get("X-Lmm-User-Credential") != "root-session" {
				t.Error("config update loses root Session")
				w.WriteHeader(403)
				return
			}
			var in market.LegacyMarketConfigUpdate
			decoder := json.NewDecoder(r.Body)
			decoder.DisallowUnknownFields()
			if decoder.Decode(&in) != nil || in.OperationID != "op" || !in.Enabled || in.FeeBPS != 100 || in.RecipientID != 3 {
				t.Error("config update DTO changed")
				w.WriteHeader(400)
				return
			}
			json.NewEncoder(w).Encode(market.LegacyMarketConfigUpdateReceipt{LegacyMarketConfig: market.LegacyMarketConfig{Enabled: in.Enabled, FeeBPS: in.FeeBPS, RecipientID: in.RecipientID, UpdatedBy: 3, UpdatedAt: 100, Version: 2}, OperationID: in.OperationID})
		default:
			t.Error("unexpected config endpoint", r.URL.Path)
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
	got, e := a.ReadLegacyConfig(context.Background())
	if e != nil || got.Enabled || got.Version != 1 || got.RecipientID != 0 {
		t.Fatalf("%+v %v", got, e)
	}
	out, e := a.UpdateLegacyConfig(context.Background(), "root-session", market.LegacyMarketConfigUpdate{OperationID: "op", Enabled: true, FeeBPS: 100, RecipientID: 3})
	if e != nil || out.OperationID != "op" || out.UpdatedBy != 3 || out.Version != 2 {
		t.Fatalf("%+v %v", out, e)
	}
	mu.Lock()
	bad = true
	mu.Unlock()
	if _, e = a.ReadLegacyConfig(context.Background()); !errors.Is(e, market.ErrUnavailable) {
		t.Fatal("missing enabled decoded as false", e)
	}
	mu.Lock()
	defer mu.Unlock()
	if reads != 2 || updates != 1 {
		t.Fatalf("calls read=%d update=%d", reads, updates)
	}
}
