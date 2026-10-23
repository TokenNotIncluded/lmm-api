package assistant_services

import (
	"bytes"
	"context"
	"database/sql"
	"encoding/json"
	"fmt"
	"os"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/TokenNotIncluded/lmm-api/services/extensions/internal/modules/assistant"
	"github.com/TokenNotIncluded/lmm-api/services/extensions/internal/modules/assistant/access"
	_ "github.com/lib/pq"
)

// These tests exercise real PostgreSQL persistence and runtime privileges. The
// typed identity/confirmation boundaries are synthetic: Core and browser human
// confirmation require their separate process acceptance suite.
type pgServiceAuthority struct{}

func (pgServiceAuthority) Check(_ context.Context, credential string, _ access.Account, _ access.Permission) (access.Principal, error) {
	id, level := int64(101), int32(1)
	switch credential {
	case "session-user-101", "session-user-101-new":
	case "session-user-202":
		id = 202
	case "session-admin-502":
		id = 502
		level = 5
	case "session-admin-501":
		id = 501
		level = 5
	default:
		return access.Principal{}, access.ErrUnauthorized
	}
	return access.Principal{UserID: id, Level: level, Account: access.Account{Kind: "personal", ID: id}}, nil
}

type pgServiceConfirm struct {
	mu       sync.Mutex
	approved bool
	calls    map[string]bool
}

func (p *pgServiceConfirm) Prepare(_ context.Context, _ string, operation, tool string, _ string, payload json.RawMessage) (Challenge, error) {
	p.mu.Lock()
	defer p.mu.Unlock()
	if p.calls == nil {
		p.calls = map[string]bool{}
	}
	p.calls[operation] = true
	return Challenge{ID: operation, URL: "/test-browser/" + operation, PayloadSHA256: digest(payload), ExpiresAt: time.Now().Unix() + 300}, nil
}
func (p *pgServiceConfirm) Verify(_ context.Context, credential, challenge, operation, sha string) (BrowserProof, error) {
	p.mu.Lock()
	defer p.mu.Unlock()
	principal, e := (pgServiceAuthority{}).Check(context.Background(), credential, access.Account{}, access.Read)
	if e != nil {
		return BrowserProof{}, e
	}
	return BrowserProof{Confirmed: p.approved && p.calls[operation] && challenge == operation, UserID: principal.UserID, OperationID: operation, PayloadSHA256: sha, ReceiptID: "synthetic-browser-proof", ExpiresAt: time.Now().Unix() + 300}, nil
}

var pgServiceOperation atomic.Uint64

func pgServiceInvocation(credential, name, arguments string) assistant.AgentInvocation {
	p, _ := (pgServiceAuthority{}).Check(context.Background(), credential, access.Account{}, access.Read)
	return assistant.AgentInvocation{Credential: credential, Actor: p, OriginalUserText: "Please remember my project preference", Call: assistant.AgentToolCall{OperationID: fmt.Sprintf("services-pg-operation-%012d", pgServiceOperation.Add(1)), CoreConversationID: "core-owned-conversation", Scope: assistant.AgentToolScope{Name: name, Version: assistant.FixedMainToolVersion, SchemaSHA256: strings.Repeat("b", 64), PriceVersion: assistant.BuiltinToolPriceVersion, PriceSHA256: assistant.FreeAgentPriceSHA256()}, Arguments: json.RawMessage(arguments), ArgumentsSHA256: digest([]byte(arguments))}}
}
func pgServiceDB(t *testing.T, key string) *sql.DB {
	t.Helper()
	path := os.Getenv(key)
	if path == "" {
		t.Skip("new isolated assistant services PG fixture required")
	}
	info, e := os.Lstat(path)
	if e != nil || !info.Mode().IsRegular() || info.Mode().Perm() != 0600 {
		t.Fatal("private fixture credential file required")
	}
	raw, e := os.ReadFile(path)
	if e != nil {
		t.Fatal("cannot read private fixture credential")
	}
	db, e := sql.Open("postgres", strings.TrimSpace(string(raw)))
	if e != nil {
		t.Fatal("cannot open fixture database")
	}
	db.SetMaxOpenConns(8)
	t.Cleanup(func() { db.Close() })
	if e = db.Ping(); e != nil {
		t.Fatal("cannot reach isolated database")
	}
	return db
}
func pgServiceMust(t *testing.T, s *Service, in assistant.AgentInvocation) assistant.AgentToolResult {
	t.Helper()
	v, e := s.Execute(context.Background(), in)
	if e != nil {
		t.Fatalf("%s: %v", in.Call.Scope.Name, e)
	}
	return v
}
func pgServiceCount(t *testing.T, db *sql.DB, query string, args ...any) int {
	t.Helper()
	var count int
	if e := db.QueryRow(query, args...).Scan(&count); e != nil {
		t.Fatal(e)
	}
	return count
}
func pgServiceConfirmationID(t *testing.T, out assistant.AgentToolResult) string {
	t.Helper()
	var v struct {
		ID string `json:"confirmation_id"`
	}
	if json.Unmarshal(out.ResultJSON, &v) != nil || len(v.ID) != 64 {
		t.Fatalf("missing pending confirmation: %s", out.ResultJSON)
	}
	return v.ID
}
func TestServiceRestrictedPG(t *testing.T) {
	ctx := context.Background()
	runtime := pgServiceDB(t, "LMM_ASSISTANT_SERVICES_PG_RUNTIME_FILE")
	owner := pgServiceDB(t, "LMM_ASSISTANT_SERVICES_PG_OWNER_FILE")
	authority := pgServiceAuthority{}
	proof := &pgServiceConfirm{}
	if _, e := New(ctx, owner, authority, Dependencies{}); e != access.ErrForbidden {
		t.Fatalf("installer accepted: %v", e)
	}
	s, e := New(ctx, runtime, authority, Dependencies{Confirmations: proof})
	if e != nil {
		t.Fatalf("restricted runtime refused: %v", e)
	}
	t.Run("memory_persistence_replay_privacy", func(t *testing.T) {
		in := pgServiceInvocation("session-user-101", "remember_memory", `{"title":"Project preference","content":"Use concise Chinese answers","tags":["language"]}`)
		first := pgServiceMust(t, s, in)
		second := pgServiceMust(t, s, in)
		if !bytes.Equal(first.ResultJSON, second.ResultJSON) || first.ResultSHA256 != second.ResultSHA256 {
			t.Fatal("receipt replay changed exact result bytes")
		}
		if pgServiceCount(t, runtime, `SELECT count(*) FROM lmm_assistant.memories WHERE user_id=101`) != 1 {
			t.Fatal("replay inserted memory")
		}
		secondDB := pgServiceDB(t, "LMM_ASSISTANT_SERVICES_PG_RUNTIME_FILE")
		reopened, e := New(ctx, secondDB, authority, Dependencies{})
		if e != nil {
			t.Fatal(e)
		}
		persisted, e := reopened.Inspect(ctx, in)
		if e != nil || !bytes.Equal(persisted.ResultJSON, first.ResultJSON) {
			t.Fatal("cross-pool receipt lost", e)
		}
		own := pgServiceMust(t, reopened, pgServiceInvocation("session-user-101", "recall_memory", `{"query":"Chinese"}`))
		if !bytes.Contains(own.ResultJSON, []byte("Use concise Chinese answers")) {
			t.Fatal("saved memory absent")
		}
		other := pgServiceMust(t, reopened, pgServiceInvocation("session-user-202", "recall_memory", `{}`))
		if bytes.Contains(other.ResultJSON, []byte("Project preference")) {
			t.Fatal("cross-user memory leak")
		}
		changed := in
		changed.Call.Arguments = json.RawMessage(`{"title":"Project preference","content":"other"}`)
		changed.Call.ArgumentsSHA256 = digest(changed.Call.Arguments)
		if _, e := s.Execute(ctx, changed); e != access.ErrConflict {
			t.Fatal("changed operation replay accepted", e)
		}
		changed = in
		changed.Credential = "session-user-101-new"
		if _, e := s.Inspect(ctx, changed); e != access.ErrConflict {
			t.Fatal("cross-session operation replay accepted", e)
		}
		changed = in
		changed.Credential = "session-user-202"
		changed.Actor, _ = authority.Check(ctx, changed.Credential, access.Account{}, access.Read)
		if _, e := s.Inspect(ctx, changed); e != access.ErrConflict {
			t.Fatal("cross-user operation replay accepted", e)
		}
		if _, e := runtime.Exec(`UPDATE lmm_assistant.service_operations SET result_bytes='x' WHERE operation_id=$1`, in.Call.OperationID); e == nil {
			t.Fatal("runtime rewrote immutable byte receipt")
		}
		if _, e := runtime.Exec(`DELETE FROM lmm_assistant.service_operations WHERE operation_id=$1`, in.Call.OperationID); e == nil {
			t.Fatal("runtime deleted receipt")
		}
		if _, e := runtime.Exec(`CREATE TABLE lmm_assistant.forbidden(value text)`); e == nil {
			t.Fatal("runtime created schema object")
		}
	})
	t.Run("administrator_profile_is_preserved", func(t *testing.T) {
		out := pgServiceMust(t, s, pgServiceInvocation("session-user-202", "remember_profile_skill", `{"profile_key":"normal_user"}`))
		if out.State != assistant.AgentToolFailed || !bytes.Contains(out.ResultJSON, []byte("administrator_managed")) {
			t.Fatal("administrator profile overwritten")
		}
		in := pgServiceInvocation("session-user-202", "forget_profile_skill", `{"confirm":true}`)
		in.OriginalUserText = "Please forget my profile"
		out = pgServiceMust(t, s, in)
		if out.State != assistant.AgentToolFailed || !bytes.Contains(out.ResultJSON, []byte("administrator_managed")) {
			t.Fatal("administrator profile deleted")
		}
		if pgServiceCount(t, runtime, `SELECT count(*) FROM lmm_assistant.profiles WHERE user_id=202 AND source='administrator' AND profile_key='privacy_conscious'`) != 1 {
			t.Fatal("administrator profile changed")
		}
	})
	t.Run("confirmation_payload_privacy_atomicity_and_cas", func(t *testing.T) {
		create := pgServiceInvocation("session-user-101", "create_site_issue", `{"kind":"security","title":"Security report","body":"Synthetic private vulnerability details","visibility":"user"}`)
		pending := pgServiceMust(t, s, create)
		id := pgServiceConfirmationID(t, pending)
		if pgServiceCount(t, runtime, `SELECT count(*) FROM lmm_assistant.site_issues`) != 0 {
			t.Fatal("issue created before browser proof")
		}
		if _, e := s.ConfirmAfterProof(ctx, create.Credential, id); e != access.ErrForbidden {
			t.Fatal("unapproved confirmation accepted", e)
		}
		var payload []byte
		var sha string
		if e := runtime.QueryRow(`SELECT payload_bytes,payload_sha256 FROM lmm_assistant.service_confirmations WHERE id=$1`, id).Scan(&payload, &sha); e != nil || digest(payload) != sha {
			t.Fatal("payload bytes/hash not preserved", e)
		}
		if _, e := runtime.Exec(`UPDATE lmm_assistant.service_confirmations SET payload_bytes='x' WHERE id=$1`, id); e == nil {
			t.Fatal("runtime modified immutable preview bytes")
		}
		if _, e := s.Confirmation(ctx, "session-user-202", id); e != access.ErrNotFound {
			t.Fatal("cross-user confirmation leaked", e)
		}
		proof.mu.Lock()
		proof.approved = true
		proof.mu.Unlock()
		var wg sync.WaitGroup
		errs := make(chan error, 8)
		for i := 0; i < 8; i++ {
			wg.Add(1)
			go func() { defer wg.Done(); _, e := s.ConfirmAfterProof(ctx, create.Credential, id); errs <- e }()
		}
		wg.Wait()
		close(errs)
		for e := range errs {
			if e != nil {
				t.Fatal("concurrent confirmation", e)
			}
		}
		if pgServiceCount(t, runtime, `SELECT count(*) FROM lmm_assistant.site_issues`) != 1 || pgServiceCount(t, runtime, `SELECT count(*) FROM lmm_assistant.site_issue_events`) != 1 {
			t.Fatal("confirmation duplicated effect")
		}
		var issueID int64
		if e := runtime.QueryRow(`SELECT id FROM lmm_assistant.site_issues`).Scan(&issueID); e != nil {
			t.Fatal(e)
		}
		if _, e := s.Execute(ctx, pgServiceInvocation("session-user-101", "get_site_issues", fmt.Sprintf(`{"issue_id":%d}`, issueID))); e != access.ErrNotFound {
			t.Fatal("security issue visible to reporter", e)
		}
		admin := pgServiceMust(t, s, pgServiceInvocation("session-admin-501", "get_site_issues", fmt.Sprintf(`{"issue_id":%d}`, issueID)))
		if !bytes.Contains(admin.ResultJSON, []byte("Synthetic private vulnerability")) {
			t.Fatal("administrator cannot inspect security report")
		}
		updateArgs := fmt.Sprintf(`{"issue_id":%d,"revision":1,"status":"triaged","visibility":"user","note":"Administrator reviewed"}`, issueID)
		first := pgServiceMust(t, s, pgServiceInvocation("session-admin-501", "update_site_issue", updateArgs))
		second := pgServiceMust(t, s, pgServiceInvocation("session-admin-501", "update_site_issue", updateArgs))
		firstID, secondID := pgServiceConfirmationID(t, first), pgServiceConfirmationID(t, second)
		if _, e := s.ConfirmAfterProof(ctx, "session-admin-501", firstID); e != nil {
			t.Fatal(e)
		}
		if _, e := s.ConfirmAfterProof(ctx, "session-admin-501", secondID); e != access.ErrConflict {
			t.Fatal("stale revision confirmation accepted", e)
		}
		if pgServiceCount(t, runtime, `SELECT count(*) FROM lmm_assistant.site_issue_events WHERE issue_id=$1`, issueID) != 2 {
			t.Fatal("failed CAS appended event")
		}
		if pgServiceCount(t, runtime, `SELECT count(*) FROM lmm_assistant.service_confirmations WHERE id=$1 AND consumed_at IS NULL`, secondID) != 1 {
			t.Fatal("failed CAS consumed confirmation")
		}
		if pgServiceCount(t, runtime, `SELECT count(*) FROM lmm_assistant.site_issues WHERE id=$1 AND revision=2 AND visibility='admin'`, issueID) != 1 {
			t.Fatal("security visibility/CAS invariant failed")
		}
	})
}

type pgCountedSearch struct {
	calls   atomic.Int64
	unknown bool
}

func (p *pgCountedSearch) Search(_ context.Context, _ string) (json.RawMessage, error) {
	p.calls.Add(1)
	if p.unknown {
		return nil, &SearchError{Unknown: true}
	}
	return json.RawMessage(`{"ok":true,"results":[{"title":"Synthetic result","text":"Current local fixture result"}]}`), nil
}
func TestServiceSearchAndSupportPG(t *testing.T) {
	ctx := context.Background()
	runtime := pgServiceDB(t, "LMM_ASSISTANT_SERVICES_PG_RUNTIME_FILE")
	authority := pgServiceAuthority{}
	t.Run("search_durable_marker_success_and_unknown", func(t *testing.T) {
		provider := &pgCountedSearch{}
		s, e := New(ctx, runtime, authority, Dependencies{Search: provider})
		if e != nil {
			t.Fatal(e)
		}
		in := pgServiceInvocation("session-user-101", "search_web", `{"query":"official installation"}`)
		first := pgServiceMust(t, s, in)
		if first.State != assistant.AgentToolSucceeded || provider.calls.Load() != 1 {
			t.Fatal("initial search did not execute once")
		}
		otherDB := pgServiceDB(t, "LMM_ASSISTANT_SERVICES_PG_RUNTIME_FILE")
		reopened, e := New(ctx, otherDB, authority, Dependencies{Search: provider})
		if e != nil {
			t.Fatal(e)
		}
		replayed := pgServiceMust(t, reopened, in)
		inspected, e := reopened.Inspect(ctx, in)
		if e != nil || !bytes.Equal(first.ResultJSON, replayed.ResultJSON) || !bytes.Equal(first.ResultJSON, inspected.ResultJSON) || provider.calls.Load() != 1 {
			t.Fatal("successful durable search reissued or changed", e)
		}
		unknownProvider := &pgCountedSearch{unknown: true}
		unknown, e := New(ctx, runtime, authority, Dependencies{Search: unknownProvider})
		if e != nil {
			t.Fatal(e)
		}
		pending := pgServiceInvocation("session-user-101", "search_web", `{"query":"unknown network response"}`)
		out := pgServiceMust(t, unknown, pending)
		if out.State != assistant.AgentToolUnknown {
			t.Fatal("ambiguous dispatch claimed terminal success")
		}
		if pgServiceCount(t, runtime, `SELECT count(*) FROM lmm_assistant.service_network_dispatch WHERE operation_id=$1`, pending.Call.OperationID) != 1 || pgServiceCount(t, runtime, `SELECT count(*) FROM lmm_assistant.service_operations WHERE operation_id=$1`, pending.Call.OperationID) != 0 {
			t.Fatal("unknown dispatch receipt is not marker-only")
		}
		restarted, e := New(ctx, otherDB, authority, Dependencies{Search: unknownProvider})
		if e != nil {
			t.Fatal(e)
		}
		if out = pgServiceMust(t, restarted, pending); out.State != assistant.AgentToolUnknown || unknownProvider.calls.Load() != 1 {
			t.Fatal("unknown dispatch reissued after reconnect")
		}
		out, e = restarted.Inspect(ctx, pending)
		if e != nil || out.State != assistant.AgentToolUnknown || unknownProvider.calls.Load() != 1 {
			t.Fatal("unknown inspect reissued request", e)
		}
		for _, credential := range []string{"session-user-202", "session-user-101-new"} {
			changed := pending
			changed.Credential = credential
			changed.Actor, _ = authority.Check(ctx, credential, access.Account{}, access.Read)
			if _, e := restarted.Execute(ctx, changed); e != access.ErrConflict {
				t.Fatalf("unknown marker cross-binding %s accepted: %v", credential, e)
			}
			if _, e := restarted.Inspect(ctx, changed); e != access.ErrConflict {
				t.Fatalf("unknown marker inspect cross-binding %s accepted: %v", credential, e)
			}
		}
		changed := pending
		changed.Call.Arguments = json.RawMessage(`{"query":"changed dispatch parameters"}`)
		changed.Call.ArgumentsSHA256 = digest(changed.Call.Arguments)
		if _, e := restarted.Execute(ctx, changed); e != access.ErrConflict {
			t.Fatal("unknown operation arguments replaced", e)
		}
		if unknownProvider.calls.Load() != 1 {
			t.Fatal("conflicting replays reached search port")
		}
		if _, e := runtime.Exec(`UPDATE lmm_assistant.service_network_dispatch SET created_at=0 WHERE operation_id=$1`, pending.Call.OperationID); e == nil {
			t.Fatal("runtime changed dispatch marker")
		}
	})
	t.Run("human_support_assignment_messages_pause_and_close", func(t *testing.T) {
		proof := &pgServiceConfirm{approved: true}
		s, e := New(ctx, runtime, authority, Dependencies{Confirmations: proof})
		if e != nil {
			t.Fatal(e)
		}
		in := pgServiceInvocation("session-user-101", "request_human_support", `{"message":"Please arrange a human support handoff","action":"support"}`)
		if active, e := s.HumanSupportActive(ctx, in.Credential, in.Call.CoreConversationID); e != nil || active {
			t.Fatal("AI paused before support request", e)
		}
		pending := pgServiceMust(t, s, in)
		id := pgServiceConfirmationID(t, pending)
		if pgServiceCount(t, runtime, `SELECT count(*) FROM lmm_assistant.platform_support`) != 0 {
			t.Fatal("handoff created without confirmation")
		}
		if _, e := s.ConfirmAfterProof(ctx, in.Credential, id); e != nil {
			t.Fatal(e)
		}
		inspected, e := s.Inspect(ctx, in)
		if e != nil || inspected.State != assistant.AgentToolSucceeded {
			t.Fatal("confirmed handoff still pending", e)
		}
		if _, e := s.SupportQueue(ctx, in.Credential, 0); e != access.ErrForbidden {
			t.Fatal("customer read administrator queue", e)
		}
		queue, e := s.SupportQueue(ctx, "session-admin-501", 0)
		if e != nil || len(queue) != 1 || queue[0].Status != "pending" {
			t.Fatal("confirmed request absent from actual queue", e)
		}
		supportID := queue[0].ID
		if pgServiceCount(t, runtime, `SELECT count(*) FROM lmm_assistant.platform_support_todos WHERE support_id=$1 AND state='pending'`, supportID) != 1 {
			t.Fatal("handoff has no pending inbox task")
		}
		if active, e := s.HumanSupportActive(ctx, in.Credential, in.Call.CoreConversationID); e != nil || !active {
			t.Fatal("AI did not pause for handoff", e)
		}
		if active, e := s.HumanSupportActive(ctx, in.Credential, "another-conversation"); e != nil || active {
			t.Fatal("handoff paused unrelated conversation", e)
		}
		if _, e := s.SupportMessages(ctx, "session-admin-502", supportID, 0); e != access.ErrForbidden {
			t.Fatal("unassigned administrator read transcript", e)
		}
		assigned, e := s.AcceptSupport(ctx, "session-admin-501", supportID)
		if e != nil || assigned.AssignedAdminID == nil || *assigned.AssignedAdminID != 501 || assigned.Status != "accepted" {
			t.Fatal("administrator assignment failed", e)
		}
		if _, e := s.AcceptSupport(ctx, "session-admin-502", supportID); e != access.ErrConflict {
			t.Fatal("second administrator stole assignment", e)
		}
		if _, e := s.SendSupportMessage(ctx, "session-admin-502", supportID, "unassigned-turn", "private reply"); e != access.ErrForbidden {
			t.Fatal("unassigned administrator sent transcript message", e)
		}
		human, e := s.SendSupportMessage(ctx, "session-admin-501", supportID, "human-reply-00001", "Please explain your installation problem")
		if e != nil || human.Role != "human" {
			t.Fatal("assigned administrator send failed", e)
		}
		replay, e := s.SendSupportMessage(ctx, "session-admin-501", supportID, "human-reply-00001", "Please explain your installation problem")
		if e != nil || replay.ID != human.ID {
			t.Fatal("message retry duplicated effect", e)
		}
		if _, e := s.SendSupportMessage(ctx, "session-admin-501", supportID, "human-reply-00001", "conflicting changed content"); e != access.ErrConflict {
			t.Fatal("message idempotency key replaced content", e)
		}
		customer, e := s.SendSupportMessage(ctx, in.Credential, supportID, "customer-reply-00001", "My local installer reports an error")
		if e != nil || customer.Role != "user" {
			t.Fatal("customer send failed", e)
		}
		messages, e := s.SupportMessages(ctx, in.Credential, supportID, 0)
		if e != nil || len(messages) != 2 {
			t.Fatal("customer transcript missing or duplicated", e)
		}
		if _, e := s.SupportMessages(ctx, "session-user-202", supportID, 0); e != access.ErrForbidden {
			t.Fatal("other customer read transcript", e)
		}
		if _, e := s.SupportMessages(ctx, "session-admin-502", supportID, 0); e != access.ErrForbidden {
			t.Fatal("unassigned administrator read assigned transcript", e)
		}
		if _, e := s.CloseSupport(ctx, in.Credential, supportID, false); e != access.ErrForbidden {
			t.Fatal("customer finalized administrator request", e)
		}
		closed, e := s.CloseSupport(ctx, in.Credential, supportID, true)
		if e != nil || closed.Status != "cancelled" || closed.ClosedAt == nil {
			t.Fatal("customer cancellation failed", e)
		}
		if active, e := s.HumanSupportActive(ctx, in.Credential, in.Call.CoreConversationID); e != nil || active {
			t.Fatal("AI did not resume after close", e)
		}
		if pgServiceCount(t, runtime, `SELECT count(*) FROM lmm_assistant.platform_support_todos WHERE support_id=$1 AND state='closed'`, supportID) != 1 {
			t.Fatal("closed request inbox not closed")
		}
		for _, credential := range []string{in.Credential, "session-admin-501"} {
			if _, e := s.SendSupportMessage(ctx, credential, supportID, "late-message-00001", "must not be written after close"); e != access.ErrConflict {
				t.Fatal("closed request accepted late message", e)
			}
		}
		if pgServiceCount(t, runtime, `SELECT count(*) FROM lmm_assistant.platform_support_messages WHERE support_id=$1`, supportID) != 2 {
			t.Fatal("closed or retried write changed transcript")
		}
	})
}
