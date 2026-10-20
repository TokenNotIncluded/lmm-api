package support

import (
	"context"
	"errors"
	"net"
	"net/http"
	"path/filepath"
	"strings"
	"sync/atomic"
	"testing"

	"github.com/TokenNotIncluded/api.lmm.best/extensions/internal/modules/store"
)

func TestCoreCustomerProjectionUsesOnlyUDSAndSeparateCredentials(t *testing.T) {
	socket := filepath.Join(t.TempDir(), "core.sock")
	listener, err := net.Listen("unix", socket)
	must(t, err)
	var calls atomic.Int64
	var failure atomic.Bool
	server := &http.Server{Handler: http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		calls.Add(1)
		if r.URL.Path == "/internal/support/v1/capabilities" {
			if r.Header.Get("Authorization") != "Bearer "+strings.Repeat("h", 32) || r.Header.Get("X-Lmm-User-Credential") != "" || r.Header.Get("X-Lmm-Protocol") != "1" {
				t.Error("capability credential changed")
				w.WriteHeader(403)
				return
			}
			_, _ = w.Write([]byte(`{"protocol_major":1,"features":["support.customers.integer_credits.v1"]}`))
			return
		}
		if r.URL.Path != "/internal/support/v1/customers" || r.Method != "POST" || r.Header.Get("Authorization") != "Bearer "+strings.Repeat("h", 32) || r.Header.Get("X-Lmm-User-Credential") != sellerToken || r.Header.Get("X-Lmm-Protocol") != "1" {
			t.Error("credential or route changed")
			w.WriteHeader(403)
			return
		}
		if failure.Load() {
			w.WriteHeader(503)
			return
		}
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`[{"buyer_id":2,"display_name":"Core name","order_count":1,"paid_quota":12345,"last_order_at":10,"last_order_id":"order","conversation_id":"conversation"}]`))
	})}
	go func() { _ = server.Serve(listener) }()
	defer server.Close()
	projection, err := NewCoreCustomerProjection(socket, []byte(strings.Repeat("h", 32)))
	must(t, err)
	defer projection.Close()
	must(t, projection.Ready(context.Background()))
	rows, err := projection.Customers(context.Background(), sellerToken, 1, nil, "", 0, 30)
	must(t, err)
	if len(rows) != 1 || rows[0].PaidQuota != 12345 || rows[0].Note != "" {
		t.Fatal(rows)
	}
	failure.Store(true)
	if _, err = projection.Customers(context.Background(), sellerToken, 1, nil, "", 0, 30); !errors.Is(err, store.ErrUnavailable) {
		t.Fatal(err)
	}
	if calls.Load() != 3 {
		t.Fatal("projection read unexpectedly replayed", calls.Load())
	}
	for _, path := range []string{"http://localhost:1", "relative.sock", "/tmp/../core.sock"} {
		if _, err = NewCoreCustomerProjection(path, []byte(strings.Repeat("h", 32))); !errors.Is(err, store.ErrInvalid) {
			t.Fatal(path, err)
		}
	}
}
