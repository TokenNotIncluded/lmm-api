package bounties

import (
	"context"
	"encoding/json"
	"net/http"
	"testing"
)

type queryAuthority struct {
	*authority
	queryCalls  int
	queryResult Receipt
	queryError  error
	authCalls   int
}

func (a *queryAuthority) Authorize(ctx context.Context, token string) (Actor, error) {
	a.authCalls++
	return a.authority.Authorize(ctx, token)
}
func (a *queryAuthority) Query(_ context.Context, _, _ string) (Receipt, error) {
	a.queryCalls++
	return a.queryResult, a.queryError
}
func TestQueryOperationLocalReceiptStillAuthenticatesAndNeverReexecutes(t *testing.T) {
	s, a := setup(t)
	ctx := context.Background()
	invoke(t, s, "owner-token", "create_draft", "draft")
	invoke(t, s, "owner-token", "publish", "publish")
	q := &queryAuthority{authority: a}
	s.Funds = q
	r, e := s.QueryOperation(ctx, "owner-token", "publish")
	if e != nil || r.State != "published" || q.authCalls != 1 || q.queryCalls != 0 || a.calls != 1 {
		t.Fatalf("%+v %v", r, e)
	}
	if _, e = s.QueryOperation(ctx, "worker-token", "publish"); e != ErrForbidden {
		t.Fatal(e)
	}
	if _, e = s.QueryOperation(ctx, "revoked-token", "publish"); e != ErrForbidden {
		t.Fatal(e)
	}
	r, e = s.QueryOperation(ctx, "owner-token", "draft")
	if e != nil || r.State != "draft" {
		t.Fatal(r, e)
	}
	if a.calls != 1 {
		t.Fatal("query repeated write")
	}
}
func TestQueryOperationMissingLocalUsesOnlyCoreQuery(t *testing.T) {
	s, a := setup(t)
	q := &queryAuthority{authority: a, queryResult: Receipt{OperationID: "op", ActorUserID: "owner", State: "confirmed", Confirmation: "opaque", Expires: 9999999999}}
	s.Funds = q
	r, e := s.QueryOperation(context.Background(), "owner-token", "op")
	if e != nil || r.State != "confirmed" || q.queryCalls != 1 || a.calls != 0 {
		t.Fatalf("%+v %v", r, e)
	}
	q.queryError = ErrNotFound
	if _, e = s.QueryOperation(context.Background(), "owner-token", "missing"); e != ErrNotFound {
		t.Fatal(e)
	}
	if a.calls != 0 {
		t.Fatal("missing query triggered write")
	}
	q.queryError = nil
	q.queryResult.ActorUserID = "worker"
	if _, e = s.QueryOperation(context.Background(), "owner-token", "op"); e != ErrForbidden {
		t.Fatal(e)
	}
}
func TestCoreClientQueryPendingConfirmedExpiredAndTerminal(t *testing.T) {
	for _, state := range []string{"awaiting_confirmation", "confirmed", "confirmation_expired", "approved"} {
		t.Run(state, func(t *testing.T) {
			c, calls := clientFixture(t, func(w http.ResponseWriter, r *http.Request) {
				if r.URL.Path != "/internal/bounties/v1/query" {
					t.Error("query made mutation request")
					w.WriteHeader(500)
					return
				}
				var payload map[string]any
				if json.NewDecoder(r.Body).Decode(&payload) != nil || len(payload) != 1 || payload["operation_id"] != "op" {
					t.Error("query payload changed")
				}
				out := map[string]any{"operation_id": "op", "actor_user_id": 71, "state": state, "status": state, "project_id": "p"}
				if state != "approved" {
					out["confirmation"] = "opaque"
					out["expires"] = 9999999999
				}
				json.NewEncoder(w).Encode(out)
			})
			r, e := c.Query(context.Background(), "session-private", "op")
			if e != nil || r.State != state || r.ActorUserID != "71" || calls.Load() != 1 {
				t.Fatalf("%+v %v", r, e)
			}
			if state != "approved" && r.ConfirmationURL != "/core/v1/bounties/confirm/opaque" {
				t.Fatal(r)
			}
		})
	}
}
func TestCoreClientMissingOrUnknownQueryNeverTriggersWrite(t *testing.T) {
	for _, status := range []int{404, 503} {
		t.Run(http.StatusText(status), func(t *testing.T) {
			c, calls := clientFixture(t, func(w http.ResponseWriter, r *http.Request) {
				if r.URL.Path != "/internal/bounties/v1/query" {
					t.Error("not query")
				}
				w.WriteHeader(status)
			})
			_, e := c.Query(context.Background(), "session-private", "op")
			want := ErrNotFound
			if status == 503 {
				want = ErrUnavailable
			}
			if e != want || calls.Load() != 1 {
				t.Fatal(e, calls.Load())
			}
		})
	}
}

func TestCoreClientRecoveryQueriesExactExpectedCommandOnly(t *testing.T) {
	cmd := Command{OperationID: "op", Action: "tip", ProjectID: "p", ChallengeID: "ch", Credit: 7, ReviewNote: "thank you", Confirmation: "old-browser-token"}
	c, calls := clientFixture(t, func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/internal/bounties/v1/query" {
			t.Error("recovery mutation")
		}
		var body struct {
			OperationID     string  `json:"operation_id"`
			ExpectedCommand Command `json:"expected_command"`
		}
		if json.NewDecoder(r.Body).Decode(&body) != nil || body.OperationID != "op" || body.ExpectedCommand != cmd {
			t.Error("expected command lost fields")
		}
		json.NewEncoder(w).Encode(map[string]any{"operation_id": "op", "project_id": "p", "challenge_id": "ch", "actor_user_id": 71, "state": "approved"})
	})
	r, e := c.QueryForCommand(context.Background(), "session-private", cmd)
	if e != nil || r.State != "approved" || calls.Load() != 1 {
		t.Fatal(r, e, calls.Load())
	}
}
