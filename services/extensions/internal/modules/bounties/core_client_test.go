package bounties

import (
	"context"
	"encoding/json"
	"net"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"sync/atomic"
	"testing"
)

func clientFixture(t *testing.T, h http.HandlerFunc) (*CoreClient, *atomic.Int32) {
	t.Helper()
	dir, e := os.MkdirTemp("", "bounty-uds-")
	if e != nil {
		t.Fatal(e)
	}
	socket := filepath.Join(dir, "s")
	listener, e := net.Listen("unix", socket)
	if e != nil {
		t.Fatal(e)
	}
	var calls atomic.Int32
	srv := &http.Server{Handler: http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		calls.Add(1)
		if r.Header.Get("Authorization") != "Bearer "+strings.Repeat("s", 32) || r.Header.Get("X-Lmm-Protocol") != "1" || (r.URL.Path != "/internal/bounties/v1/config" && r.URL.Path != "/internal/bounties/v1/ready" && r.Header.Get("X-Lmm-User-Credential") != "session-private") || ((r.URL.Path == "/internal/bounties/v1/config" || r.URL.Path == "/internal/bounties/v1/ready") && r.Header.Get("X-Lmm-User-Credential") != "") {
			t.Error("missing credential headers")
			w.WriteHeader(403)
			return
		}
		h(w, r)
	})}
	go srv.Serve(listener)
	t.Cleanup(func() { srv.Close(); os.RemoveAll(dir) })
	c, e := NewCoreClient(socket, strings.Repeat("s", 32))
	if e != nil {
		t.Fatal(e)
	}
	return c, &calls
}
func TestCoreClientSuccessfulTypedSessionPreviewAndExecute(t *testing.T) {
	c, calls := clientFixture(t, func(w http.ResponseWriter, r *http.Request) {
		var payload map[string]any
		json.NewDecoder(r.Body).Decode(&payload)
		if _, ok := payload["token"]; ok {
			t.Error("token in JSON")
		}
		switch r.URL.Path {
		case "/internal/bounties/v1/authorize":
			json.NewEncoder(w).Encode(map[string]any{"user_id": 71, "admin": true})
		case "/internal/bounties/v1/preview":
			json.NewEncoder(w).Encode(map[string]any{"confirmation": "opaque-challenge", "expires": 9999999999, "credit": 500, "fee_credit": 5, "reward_credit": 495})
		case "/internal/bounties/v1/execute":
			if payload["confirmation"] != "browser-confirmed" {
				w.WriteHeader(403)
				return
			}
			json.NewEncoder(w).Encode(map[string]any{"operation_id": "op", "project_id": "p", "state": "published", "status": "published", "journal_ids": []int{99}, "actor_user_id": 71, "escrow_credit": 495, "reward_credit": 495})
		}
	})
	ctx := context.Background()
	actor, e := c.Authorize(ctx, "session-private")
	if e != nil || actor.UserID != "71" || !actor.Admin || actor.Token != "" {
		t.Fatalf("%+v %v", actor, e)
	}
	cmd := Command{OperationID: "op", ProjectID: "p", Action: "publish"}
	preview, e := c.Preview(ctx, "session-private", cmd)
	if e != nil || preview.State != "awaiting_confirmation" || preview.ConfirmationURL != "/core/v1/bounties/confirm/opaque-challenge" || preview.FeeCredit != 5 {
		t.Fatalf("%+v %v", preview, e)
	}
	if calls.Load() != 2 {
		t.Fatal("preview executed another request")
	}
	cmd.Confirmation = "opaque-challenge"
	if _, e = c.Execute(ctx, "session-private", cmd); e != ErrForbidden {
		t.Fatal("unconfirmed challenge accepted", e)
	}
	cmd.Confirmation = "browser-confirmed"
	receipt, e := c.Execute(ctx, "session-private", cmd)
	if e != nil || receipt.ActorUserID != "71" || receipt.JournalIDs[0] != 99 {
		t.Fatalf("%+v %v", receipt, e)
	}
}
func TestCoreClientUnknownExecuteNeverRetries(t *testing.T) {
	for _, mode := range []string{"disconnect", "malformed", "unavailable"} {
		t.Run(mode, func(t *testing.T) {
			c, calls := clientFixture(t, func(w http.ResponseWriter, r *http.Request) {
				switch mode {
				case "disconnect":
					conn, _, e := w.(http.Hijacker).Hijack()
					if e != nil {
						t.Error(e)
						return
					}
					conn.Close()
				case "malformed":
					w.Write([]byte("{"))
				case "unavailable":
					w.WriteHeader(503)
				}
			})
			if _, e := c.Execute(context.Background(), "session-private", Command{OperationID: "same-operation", ProjectID: "p", Action: "publish"}); e != ErrUnknown {
				t.Fatal(e)
			}
			if calls.Load() != 1 {
				t.Fatalf("retried unknown write %d", calls.Load())
			}
		})
	}
}
func TestCoreClientRejectsInvalidTransportConfiguration(t *testing.T) {
	for _, socket := range []string{"relative.sock", "https://core.example/api"} {
		if _, e := NewCoreClient(socket, strings.Repeat("s", 32)); e != ErrInvalid {
			t.Fatal(socket, e)
		}
	}
	if _, e := NewCoreClient("/tmp/socket", "short"); e != ErrInvalid {
		t.Fatal(e)
	}
}

func TestCoreClientPublicConfigUsesNoUserCredentialAndSessionAuthSeparate(t *testing.T) {
	c, _ := clientFixture(t, func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/internal/bounties/v1/config":
			json.NewEncoder(w).Encode(map[string]any{"enabled": false, "code": "policy_not_configured", "rate_basis_points": 0, "rate_percent": "0.00", "limits": map[string]int{"reward_slots_min": 1, "reward_slots_max": 100, "tip_credit_min": 1}})
		case "/internal/bounties/v1/session-authorize":
			json.NewEncoder(w).Encode(map[string]any{"user_id": 71, "admin": false})
		default:
			w.WriteHeader(403)
		}
	})
	config, e := c.PublicConfig(context.Background())
	if e != nil || config.Enabled || config.RateBasisPoints != 0 || config.RatePercent != "0.00" || config.Limits.RewardSlotsMax != 100 {
		t.Fatal(config, e)
	}
	actor, e := c.SessionAuthorize(context.Background(), "session-private")
	if e != nil || actor.UserID != "71" {
		t.Fatal(actor, e)
	}
	if _, e = c.Authorize(context.Background(), "session-private"); e != ErrForbidden {
		t.Fatal("private developer gate bypassed", e)
	}
}

func TestCoreReadyRequiresExactLivePolicyACK(t *testing.T) {
	for _, body := range []string{`{"protocol":1,"service":"bounties","ready":true,"policy_configured":true}`, `{"protocol":1,"service":"bounties","ready":true}`, `{"protocol":2,"service":"bounties","ready":true,"policy_configured":true}`, `{"protocol":1,"service":"wallet","ready":true,"policy_configured":true}`, `{"protocol":1,"service":"bounties","ready":false,"policy_configured":true}`, `{"protocol":1,"service":"bounties","ready":true,"policy_configured":false}`, `{"protocol":1,"service":"bounties","ready":true,"policy_configured":true,"extra":true}`, `null`} {
		t.Run(body, func(t *testing.T) {
			c, _ := clientFixture(t, func(w http.ResponseWriter, r *http.Request) {
				if r.URL.Path != "/internal/bounties/v1/ready" || r.Header.Get("X-Lmm-User-Credential") != "" {
					t.Error("wrong readiness request")
				}
				w.Write([]byte(body))
			})
			e := c.Ready(context.Background())
			if body == `{"protocol":1,"service":"bounties","ready":true,"policy_configured":true}` {
				if e != nil {
					t.Fatal(e)
				}
			} else if e == nil {
				t.Fatal("accepted incomplete readiness ACK")
			}
		})
	}
	if e := ((*Service)(nil)).Ready(context.Background()); e != ErrUnavailable {
		t.Fatal(e)
	}
	if e := (&Service{}).Ready(context.Background()); e != ErrUnavailable {
		t.Fatal(e)
	}
}
