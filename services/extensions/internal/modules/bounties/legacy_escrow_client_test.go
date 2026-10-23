package bounties

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"strings"
	"sync"
	"testing"
)

func legacySubmitFixture() LegacyEscrowSubmitCommand {
	return LegacyEscrowSubmitCommand{
		MigrationID: "synthetic-legacy-v1", SourceProjectID: 9007199254740993,
		SourceChallengeID: 9007199254740995, OperationKey: "submit-original",
		PullRequestURL: "https://github.com/example/project/pull/7", SubmissionNote: "Synthetic evidence",
	}
}

func TestLegacyEscrowClientTypedCommandsAndExactQueryOverUDS(t *testing.T) {
	submit := legacySubmitFixture()
	payout := LegacyEscrowPayoutCommand{submit.MigrationID, submit.SourceProjectID, submit.SourceChallengeID, "payout-original", &submit.OperationKey}
	closeCommand := LegacyEscrowCloseCommand{submit.MigrationID, submit.SourceProjectID, "close-original"}
	c, calls := clientFixture(t, func(w http.ResponseWriter, r *http.Request) {
		for _, key := range []string{"Authorization", "X-Lmm-Protocol", "X-Lmm-User-Credential"} {
			if len(r.Header.Values(key)) != 1 {
				t.Errorf("expected exactly one %s", key)
			}
		}
		var payload map[string]json.RawMessage
		if e := json.NewDecoder(r.Body).Decode(&payload); e != nil {
			t.Error(e)
			w.WriteHeader(http.StatusBadRequest)
			return
		}
		for _, key := range []string{"amount", "payout_units", "recipient_account_id", "owner_user_id", "token", "session"} {
			if _, found := payload[key]; found {
				t.Errorf("caller chose frozen authority %s", key)
			}
		}
		if string(payload["source_project_id"]) != "9007199254740993" {
			t.Error("source ID lost integer precision")
		}
		key, status, remaining, revision := "", "", int64(0), int64(1)
		var journal *int64
		switch r.URL.Path {
		case "/internal/bounties/v1/legacy-escrow/submit":
			var command LegacyEscrowSubmitCommand
			b, _ := json.Marshal(payload)
			if decodeLegacyEscrow(b, &command) != nil || command != submit {
				t.Error("submit changed its original payload")
			}
			key, status, remaining = submit.OperationKey, "submitted", 19
		case "/internal/bounties/v1/legacy-escrow/payout":
			if string(payload["submission_operation_key"]) != `"submit-original"` || string(payload["source_challenge_id"]) != "9007199254740995" {
				t.Error("payout lost original submission binding")
			}
			id := int64(97)
			journal = &id
			key, status, remaining, revision = payout.OperationKey, "posted", 8, 2
		case "/internal/bounties/v1/legacy-escrow/close":
			if _, found := payload["source_challenge_id"]; found {
				t.Error("close contains challenge authority")
			}
			key, status = closeCommand.OperationKey, "posted"
		case "/internal/bounties/v1/legacy-escrow/query":
			var expected map[string]json.RawMessage
			if json.Unmarshal(payload["expected_command"], &expected) != nil || string(expected["kind"]) != `"submit"` || string(expected["source_challenge_id"]) != "9007199254740995" {
				t.Error("query did not preserve typed original command")
			}
			json.NewEncoder(w).Encode(map[string]any{"command": payload["expected_command"], "actor_user_id": int64(71), "receipt": LegacyEscrowReceipt{Status: "submitted", OperationKey: submit.OperationKey, RemainingUnits: 19, Revision: 1}})
			return
		default:
			t.Errorf("unexpected path %s", r.URL.Path)
			w.WriteHeader(http.StatusNotFound)
			return
		}
		json.NewEncoder(w).Encode(LegacyEscrowReceipt{Status: status, OperationKey: key, JournalID: journal, RemainingUnits: remaining, Revision: revision})
	})
	ctx := context.Background()
	if r, e := c.SubmitLegacyEscrow(ctx, "session-private", submit); e != nil || r.Status != "submitted" || r.JournalID != nil {
		t.Fatal(r, e)
	}
	o, e := c.QueryLegacyEscrowForCommand(ctx, "session-private", submit)
	if e != nil || o.Command != submit || o.ActorUserID != 71 || o.Receipt.Status != "submitted" {
		t.Fatal(o, e)
	}
	raw, e := json.Marshal(o)
	if e != nil || !strings.Contains(string(raw), `"command":{"kind":"submit"`) {
		t.Fatal("observation lost original command", string(raw), e)
	}
	if r, e := c.PayoutLegacyEscrow(ctx, "session-private", payout); e != nil || r.Status != "posted" || r.JournalID == nil || *r.JournalID != 97 {
		t.Fatal(r, e)
	}
	if r, e := c.CloseLegacyEscrow(ctx, "session-private", closeCommand); e != nil || r.Status != "posted" || r.JournalID != nil || r.RemainingUnits != 0 {
		t.Fatal("empty close must not require a journal", r, e)
	}
	if calls.Load() != 4 {
		t.Fatal("unexpected request count", calls.Load())
	}
}

func TestLegacyEscrowClientAmbiguousMutationsNeverRetry(t *testing.T) {
	for _, kind := range []string{"submit", "payout", "close"} {
		for _, mode := range []string{"disconnect", "unavailable", "malformed", "oversized", "wrong-key", "missing-revision", "unknown-status", "duplicate-receipt", "null"} {
			t.Run(kind+"/"+mode, func(t *testing.T) {
				c, calls := clientFixture(t, func(w http.ResponseWriter, r *http.Request) {
					switch mode {
					case "disconnect":
						conn, _, e := w.(http.Hijacker).Hijack()
						if e != nil {
							t.Error(e)
							return
						}
						conn.Close()
						return
					case "unavailable":
						w.WriteHeader(http.StatusServiceUnavailable)
						return
					case "malformed":
						w.Write([]byte("{"))
					case "oversized":
						w.Write([]byte(strings.Repeat(" ", 65537)))
					case "wrong-key":
						w.Write([]byte(`{"status":"posted","code":null,"operation_key":"another-key","journal_id":null,"remaining_units":0,"revision":1}`))
					case "missing-revision":
						w.Write([]byte(`{"status":"posted","operation_key":"submit-original","remaining_units":0}`))
					case "unknown-status":
						w.Write([]byte(`{"status":"probably-paid","operation_key":"submit-original","remaining_units":0,"revision":1}`))
					case "duplicate-receipt":
						w.Write([]byte(`{"status":"submitted","operation_key":"submit-original","operation_key":"other-key","remaining_units":0,"revision":1}`))
					case "null":
						w.Write([]byte("null"))
					}
				})
				submit := legacySubmitFixture()
				var e error
				switch kind {
				case "submit":
					_, e = c.SubmitLegacyEscrow(context.Background(), "session-private", submit)
				case "payout":
					_, e = c.PayoutLegacyEscrow(context.Background(), "session-private", LegacyEscrowPayoutCommand{MigrationID: submit.MigrationID, SourceProjectID: submit.SourceProjectID, SourceChallengeID: submit.SourceChallengeID, OperationKey: submit.OperationKey})
				case "close":
					_, e = c.CloseLegacyEscrow(context.Background(), "session-private", LegacyEscrowCloseCommand{submit.MigrationID, submit.SourceProjectID, submit.OperationKey})
				}
				if e != ErrUnknown || calls.Load() != 1 {
					t.Fatalf("must keep ambiguous original key: %v calls=%d", e, calls.Load())
				}
			})
		}
	}
}

func TestLegacyEscrowClientQueriesEveryCommandKind(t *testing.T) {
	submit := legacySubmitFixture()
	for _, command := range []LegacyEscrowCommand{
		submit,
		LegacyEscrowPayoutCommand{MigrationID: submit.MigrationID, SourceProjectID: submit.SourceProjectID, SourceChallengeID: submit.SourceChallengeID, OperationKey: "source-submitted-payout"},
		LegacyEscrowPayoutCommand{submit.MigrationID, submit.SourceProjectID, submit.SourceChallengeID, "new-submitted-payout", &submit.OperationKey},
		LegacyEscrowCloseCommand{submit.MigrationID, submit.SourceProjectID, "original-close"},
	} {
		kind, _, _, key := command.legacyEscrow()
		t.Run(key, func(t *testing.T) {
			c, calls := clientFixture(t, func(w http.ResponseWriter, r *http.Request) {
				if r.URL.Path != "/internal/bounties/v1/legacy-escrow/query" {
					t.Error("query executed a mutation")
				}
				var payload struct {
					Expected json.RawMessage `json:"expected_command"`
				}
				json.NewDecoder(r.Body).Decode(&payload)
				status := "posted"
				var journal *int64
				if kind == "submit" {
					status = "submitted"
				} else if kind == "payout" {
					id := int64(98)
					journal = &id
				}
				json.NewEncoder(w).Encode(map[string]any{"command": payload.Expected, "actor_user_id": 71, "receipt": LegacyEscrowReceipt{Status: status, OperationKey: key, JournalID: journal, RemainingUnits: 0, Revision: 2}})
			})
			if o, e := c.QueryLegacyEscrowForCommand(context.Background(), "session-private", command); e != nil || o.Receipt.OperationKey != key || calls.Load() != 1 {
				t.Fatal(o, e, calls.Load())
			}
		})
	}
}

func TestLegacyEscrowClientQueryOnlyObservesExactOriginalCommand(t *testing.T) {
	for _, mode := range []string{"exact", "other-key", "other-note", "other-kind", "forged-amount", "unknown-response-field", "invalid-receipt", "unavailable", "missing"} {
		t.Run(mode, func(t *testing.T) {
			command := legacySubmitFixture()
			c, calls := clientFixture(t, func(w http.ResponseWriter, r *http.Request) {
				if r.URL.Path != "/internal/bounties/v1/legacy-escrow/query" {
					t.Error("query attempted mutation")
				}
				var input struct {
					Expected map[string]json.RawMessage `json:"expected_command"`
				}
				json.NewDecoder(r.Body).Decode(&input)
				response := map[string]any{"command": input.Expected, "actor_user_id": 71, "receipt": LegacyEscrowReceipt{Status: "submitted", OperationKey: command.OperationKey, RemainingUnits: 19, Revision: 1}}
				switch mode {
				case "other-key":
					input.Expected["operation_key"] = json.RawMessage(`"a-new-key"`)
				case "other-note":
					input.Expected["submission_note"] = json.RawMessage(`"different evidence"`)
				case "other-kind":
					input.Expected["kind"] = json.RawMessage(`"close"`)
				case "forged-amount":
					input.Expected["amount_units"] = json.RawMessage("500")
				case "unknown-response-field":
					response["approved"] = true
				case "invalid-receipt":
					response["receipt"] = map[string]any{"status": "submitted", "operation_key": command.OperationKey, "remaining_units": -1, "revision": 1}
				case "unavailable":
					w.WriteHeader(http.StatusServiceUnavailable)
					return
				case "missing":
					w.WriteHeader(http.StatusNotFound)
					return
				}
				json.NewEncoder(w).Encode(response)
			})
			_, e := c.QueryLegacyEscrowForCommand(context.Background(), "session-private", command)
			want := ErrUnavailable
			switch mode {
			case "exact":
				want = nil
			case "other-key", "other-note", "other-kind":
				want = ErrConflict
			case "missing":
				want = ErrNotFound
			}
			if e != want || calls.Load() != 1 {
				t.Fatalf("query changed/retried command: got=%v want=%v calls=%d", e, want, calls.Load())
			}
		})
	}
}

func TestLegacyEscrowClientRejectsInvalidInputBeforeDial(t *testing.T) {
	c, calls := clientFixture(t, func(w http.ResponseWriter, r *http.Request) { t.Error("invalid request reached Core") })
	submit := legacySubmitFixture()
	for _, session := range []string{"", "session\nprivate", strings.Repeat("s", 4097)} {
		if _, e := c.SubmitLegacyEscrow(context.Background(), session, submit); e != ErrForbidden {
			t.Fatal(e)
		}
	}
	for _, bad := range []LegacyEscrowSubmitCommand{
		{MigrationID: "migration", SourceProjectID: 0, SourceChallengeID: 5, OperationKey: "original"},
		{MigrationID: "migration", SourceProjectID: 3, SourceChallengeID: -1, OperationKey: "original"},
		{MigrationID: "migration", SourceProjectID: 3, SourceChallengeID: 5, OperationKey: "new key"},
		{MigrationID: "migration", SourceProjectID: 3, SourceChallengeID: 5, OperationKey: "original", SubmissionNote: strings.Repeat("n", 16384)},
	} {
		if _, e := c.SubmitLegacyEscrow(context.Background(), "session-private", bad); e != ErrInvalid {
			t.Fatal(e)
		}
	}
	if calls.Load() != 0 {
		t.Fatal("invalid input dialed Core", calls.Load())
	}
}

func TestLegacyEscrowClientPreservesBusinessRejection(t *testing.T) {
	c, calls := clientFixture(t, func(w http.ResponseWriter, r *http.Request) {
		fmt.Fprint(w, `{"status":"rejected","code":"active_claim","operation_key":"close-original","journal_id":null,"remaining_units":19,"revision":1}`)
	})
	r, e := c.CloseLegacyEscrow(context.Background(), "session-private", LegacyEscrowCloseCommand{"synthetic-legacy-v1", 9, "close-original"})
	if e != nil || r.Status != "rejected" || r.Code == nil || *r.Code != "active_claim" || r.JournalID != nil || calls.Load() != 1 {
		t.Fatal(r, e, calls.Load())
	}
}

func TestLegacyEscrowClientLostAckQueriesOriginalCommandWithoutRetry(t *testing.T) {
	command := legacySubmitFixture()
	var recorded map[string]json.RawMessage
	var recordedMu sync.Mutex
	c, calls := clientFixture(t, func(w http.ResponseWriter, r *http.Request) {
		recordedMu.Lock()
		defer recordedMu.Unlock()
		switch r.URL.Path {
		case "/internal/bounties/v1/legacy-escrow/submit":
			if recorded != nil {
				t.Error("unknown mutation was automatically repeated")
			}
			json.NewDecoder(r.Body).Decode(&recorded)
			recorded["kind"] = json.RawMessage(`"submit"`)
			connection, _, e := w.(http.Hijacker).Hijack()
			if e != nil {
				t.Error(e)
				return
			}
			connection.Close()
		case "/internal/bounties/v1/legacy-escrow/query":
			if recorded == nil {
				t.Error("query preceded original request")
			}
			json.NewEncoder(w).Encode(map[string]any{"command": recorded, "actor_user_id": 71, "receipt": LegacyEscrowReceipt{Status: "submitted", OperationKey: command.OperationKey, RemainingUnits: 19, Revision: 1}})
		default:
			t.Error("unexpected request", r.URL.Path)
			w.WriteHeader(http.StatusNotFound)
		}
	})
	if _, e := c.SubmitLegacyEscrow(context.Background(), "session-private", command); e != ErrUnknown {
		t.Fatal("lost ACK must remain unknown", e)
	}
	if o, e := c.QueryLegacyEscrowForCommand(context.Background(), "session-private", command); e != nil || o.Receipt.OperationKey != command.OperationKey || o.Command != command {
		t.Fatal("query did not reconcile original command", o, e)
	}
	if calls.Load() != 2 {
		t.Fatal("unexpected retry or operation", calls.Load())
	}
}

func TestLegacyEscrowClientRejectsAliasedReceiptFields(t *testing.T) {
	for _, body := range []string{
		`{"status":"submitted","operation_key":"forged","Operation_Key":"submit-original","remaining_units":19,"revision":1}`,
		`{"Status":"submitted","operation_key":"submit-original","remaining_units":19,"revision":1}`,
		`{"status":"submitted","operation_key":"submit-original","remaining_units":19,"Remaining_Units":23,"revision":1}`,
		`{"ſtatus":"submitted","operation_key":"submit-original","remaining_units":19,"revision":1}`,
		`{"status":"submitted","operation_Key":"submit-original","remaining_units":19,"revision":1}`,
	} {
		t.Run(body, func(t *testing.T) {
			c, calls := clientFixture(t, func(w http.ResponseWriter, r *http.Request) { fmt.Fprint(w, body) })
			if _, e := c.SubmitLegacyEscrow(context.Background(), "session-private", legacySubmitFixture()); e != ErrUnknown || calls.Load() != 1 {
				t.Fatal("aliased receipt changed original observation", e, calls.Load())
			}
		})
	}
}

func TestLegacyEscrowClientRejectsAliasedQueryEnvelopeAndNestedFields(t *testing.T) {
	command := legacySubmitFixture()
	tagged, _ := legacyEscrowTagged(command)
	rawCommand, _ := json.Marshal(tagged)
	original := fmt.Sprintf(`{"command":%s,"actor_user_id":71,"receipt":{"status":"submitted","operation_key":"submit-original","remaining_units":19,"revision":1}}`, rawCommand)
	for _, change := range [][2]string{
		{`"actor_user_id":71`, `"Actor_User_Id":71`},
		{`"actor_user_id":71`, `"actor_user_id":0,"Actor_User_Id":71`},
		{`"status":"submitted"`, `"Status":"submitted"`},
		{`"kind":"submit"`, `"Kind":"submit"`},
		{`"kind":"submit"`, `"Kind":"submit"`},
		{`"status":"submitted"`, `"ſtatus":"submitted"`},
		{`"source_project_id":9007199254740993`, `"Source_Project_Id":9007199254740993`},
		{`"submission_note":"Synthetic evidence"`, `"Submission_Note":"Synthetic evidence"`},
		{`"operation_key":"submit-original"`, `"operation_key":"forged","Operation_Key":"submit-original"`},
	} {
		t.Run(change[1], func(t *testing.T) {
			body := strings.Replace(original, change[0], change[1], 1)
			if body == original {
				t.Fatal("alias fixture did not change wire input")
			}
			c, calls := clientFixture(t, func(w http.ResponseWriter, r *http.Request) { fmt.Fprint(w, body) })
			if _, e := c.QueryLegacyEscrowForCommand(context.Background(), "session-private", command); e != ErrUnavailable || calls.Load() != 1 {
				t.Fatal("query accepted aliased field", e, calls.Load())
			}
		})
	}
}
