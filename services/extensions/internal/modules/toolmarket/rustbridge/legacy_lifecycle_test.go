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

func TestLegacyLifecycleV2UDSExactAuthorityReceipt(t *testing.T) {
	root := os.Getenv("LMM_TEST_SOCKET_ROOT")
	if root == "" {
		root = t.TempDir()
	}
	socket := filepath.Join(root, "l")
	listener, e := net.Listen("unix", socket)
	if e != nil {
		t.Fatal(e)
	}
	host := strings.Repeat("h", 43)
	mu := sync.Mutex{}
	bad := false
	calls := 0
	server := &http.Server{Handler: http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		mu.Lock()
		defer mu.Unlock()
		calls++
		if r.Method != "POST" || r.URL.Path != "/internal/tool-market/v2/services/lifecycle" || r.Header.Get("Authorization") != "Bearer "+host || r.Header.Get("X-Lmm-Protocol") != "2" || r.Header.Get("X-Lmm-User-Credential") != "publisher-session" {
			t.Error("lifecycle authority transport changed")
			w.WriteHeader(403)
			return
		}
		var in market.LegacyServiceLifecycleRequest
		decoder := json.NewDecoder(r.Body)
		decoder.DisallowUnknownFields()
		if decoder.Decode(&in) != nil || in.OperationID != "pause" || in.ServiceID != "service" || !in.Paused || in.Deleted {
			t.Error("lifecycle intent changed")
			w.WriteHeader(400)
			return
		}
		if bad {
			json.NewEncoder(w).Encode(map[string]any{"operation_id": in.OperationID, "service_id": in.ServiceID, "paused": in.Paused, "deleted": in.Deleted, "revision": 2, "updated_by": 1, "updated_at": 100})
			return
		}
		json.NewEncoder(w).Encode(market.LegacyServiceLifecycleReceipt{LegacyServiceLifecycleRequest: in, Suspended: false, Revision: 2, UpdatedBy: 1, UpdatedAt: 100})
	})}
	done := make(chan error, 1)
	go func() { done <- server.Serve(listener) }()
	t.Cleanup(func() { server.Close(); <-done })
	a, e := New(socket, []byte(host))
	if e != nil {
		t.Fatal(e)
	}
	defer a.Close()
	in := market.LegacyServiceLifecycleRequest{OperationID: "pause", ServiceID: "service", Paused: true}
	out, e := a.SetLegacyServiceLifecycle(context.Background(), "publisher-session", in)
	if e != nil || out.LegacyServiceLifecycleRequest != in || out.Suspended || out.Revision != 2 || out.UpdatedBy != 1 {
		t.Fatalf("%+v %v", out, e)
	}
	mu.Lock()
	bad = true
	mu.Unlock()
	if _, e = a.SetLegacyServiceLifecycle(context.Background(), "publisher-session", in); !errors.Is(e, market.ErrUnavailable) {
		t.Fatal("missing suspended became false authority", e)
	}
	mu.Lock()
	defer mu.Unlock()
	if calls != 2 {
		t.Fatal(calls)
	}
}
