package bounties

import (
	"context"
	"database/sql"
	"encoding/json"
	"fmt"
	_ "github.com/lib/pq"
	"net/url"
	"os"
	"strings"
	"sync"
	"testing"
)

// Only a fresh, local disposable database may be supplied. This test never
// drops/replaces an existing schema and cannot use a remote site connection.
func TestPostgresPersistenceRollbackConcurrency(t *testing.T) {
	path := os.Getenv("LMM_BOUNTIES_TEST_DSN_FILE")
	if path == "" {
		t.Skip("requires fresh isolated local PostgreSQL fixture")
	}
	raw, e := os.ReadFile(path)
	if e != nil {
		t.Fatal("read private fixture")
	}
	u, e := url.Parse(strings.TrimSpace(string(raw)))
	if e != nil || u.Scheme != "postgres" || (u.Hostname() != "127.0.0.1" && u.Hostname() != "localhost" && u.Hostname() != "::1") {
		t.Fatal("fixture must be local PostgreSQL")
	}
	db, e := sql.Open("postgres", u.String())
	if e != nil {
		t.Fatal("open private fixture")
	}
	defer db.Close()
	ctx := context.Background()
	var exists bool
	var name string
	if e = db.QueryRowContext(ctx, `SELECT current_database(),EXISTS(SELECT 1 FROM pg_namespace WHERE nspname='lmm_bounties')`).Scan(&name, &exists); e != nil || exists {
		t.Fatal("fixture must be fresh")
	}
	if _, e = NewPostgres(ctx, db, name); e != ErrUnavailable {
		t.Fatal("constructor initialized missing schema")
	}
	if _, e = db.ExecContext(ctx, Schema); e != nil {
		t.Fatal("install fresh schema")
	}
	p, e := NewPostgres(ctx, db, name)
	if e != nil {
		t.Fatal(e)
	}
	if e = p.Within(ctx, func(tx Tx) error {
		return tx.Put("projects", "p", Project{ID: "p", OwnerUserID: "owner", Status: "draft"})
	}); e != nil {
		t.Fatal(e)
	}
	if e = p.Within(ctx, func(tx Tx) error { tx.Put("projects", "rollback", Project{ID: "rollback"}); return ErrConflict }); e != ErrConflict {
		t.Fatal(e)
	}
	restored, e := NewPostgres(ctx, db, name)
	if e != nil {
		t.Fatal(e)
	}
	if e = restored.Within(ctx, func(tx Tx) error {
		var p Project
		if e := tx.Get("projects", "rollback", &p); e != ErrNotFound {
			return ErrConflict
		}
		return tx.Get("projects", "p", &p)
	}); e != nil {
		t.Fatal(e)
	}
	var wg sync.WaitGroup
	errs := make(chan error, 8)
	for range 8 {
		wg.Add(1)
		go func() {
			defer wg.Done()
			errs <- restored.Within(ctx, func(tx Tx) error {
				var n struct {
					Count int `json:"count"`
				}
				e := tx.Get("operations", "counter", &n)
				if e != nil && e != ErrNotFound {
					return e
				}
				n.Count++
				return tx.Put("operations", "counter", n)
			})
		}()
	}
	wg.Wait()
	close(errs)
	for e := range errs {
		if e != nil {
			t.Fatal(e)
		}
	}
	if e = p.Within(ctx, func(tx Tx) error {
		var n struct {
			Count int `json:"count"`
		}
		if e := tx.Get("operations", "counter", &n); e != nil {
			return e
		}
		if n.Count != 8 {
			return ErrConflict
		}
		items, e := tx.List("projects", Query{OwnerUserID: "owner", Limit: 50})
		if e != nil {
			return e
		}
		if len(items) != 1 || !json.Valid(items[0]) {
			return ErrConflict
		}
		for i, id := range []string{"zzz", "aaa", "mmm"} {
			if e = tx.Put("projects", id, Project{ID: id, OwnerUserID: "board", Status: "published", PublishedAt: int64(100 + i)}); e != nil {
				return e
			}
		}
		page, e := tx.List("projects", Query{Public: true, Limit: 1, Offset: 1})
		if e != nil {
			return e
		}
		var second Project
		if len(page) != 1 || json.Unmarshal(page[0], &second) != nil || second.ID != "aaa" {
			return ErrConflict
		}
		page, e = tx.List("projects", Query{Public: true, Limit: 1, After: "mmm"})
		if e != nil {
			return e
		}
		if len(page) != 1 || json.Unmarshal(page[0], &second) != nil || second.ID != "aaa" {
			return ErrConflict
		}
		for i := 0; i < 100; i++ {
			id := fmt.Sprintf("dispute-%03d", i)
			if e = tx.Put("disputes", id, Dispute{ID: id, Status: "open", OwnerUserID: "board"}); e != nil {
				return e
			}
		}
		page, e = tx.List("disputes", Query{PartyUserID: "board", Limit: 100})
		if e != nil {
			return e
		}
		if len(page) != 100 {
			return ErrConflict
		}
		if e = tx.Put("disputes", "resolved-a", Dispute{ID: "resolved-a", Status: "resolved_paid", OwnerUserID: "board"}); e != nil {
			return e
		}
		if e = tx.Put("disputes", "resolved-b", Dispute{ID: "resolved-b", Status: "resolved_paid", OwnerUserID: "board"}); e != nil {
			return e
		}
		page, e = tx.List("disputes", Query{PartyUserID: "board", Status: "resolved_paid", Limit: 1, Offset: 1})
		if e != nil {
			return e
		}
		var d Dispute
		if len(page) != 1 || json.Unmarshal(page[0], &d) != nil || d.ID != "resolved-b" {
			return ErrConflict
		}
		return nil
	}); e != nil {
		t.Fatal(e)
	}
	testRecoverAfterActualPostgresCommitFailure(t, db, name, p)
	testPostgresRESTRecordsAndHTTP(t, p)
}

// The callback succeeds and Core has already committed its receipt. PostgreSQL
// then rejects the LOCAL COMMIT through a deferred constraint, leaving no Go
// projection. This exercises a real lost-commit boundary, not a mock repository.
type deferredCommitFailure struct{ inner *Postgres }

func (f deferredCommitFailure) Within(ctx context.Context, fn func(Tx) error) error {
	return f.inner.Within(ctx, func(tx Tx) error {
		if e := fn(tx); e != nil {
			return e
		}
		p := tx.(*postgresTx)
		_, e := p.tx.ExecContext(ctx, `INSERT INTO lmm_bounties.test_commit_child(id,parent_id) VALUES(1,999)`)
		return e
	})
}
func testRecoverAfterActualPostgresCommitFailure(t *testing.T, db *sql.DB, name string, p *Postgres) {
	t.Helper()
	ctx := context.Background()
	if _, e := db.ExecContext(ctx, `CREATE TABLE lmm_bounties.test_commit_parent(id integer PRIMARY KEY);CREATE TABLE lmm_bounties.test_commit_child(id integer PRIMARY KEY,parent_id integer REFERENCES lmm_bounties.test_commit_parent(id) DEFERRABLE INITIALLY DEFERRED)`); e != nil {
		t.Fatal(e)
	}
	a := &recoveryAuthority{authority: &authority{}}
	s := &Service{Store: p, Funds: a}
	draftIn := Invocation{Actor: Actor{Token: "owner-token"}, Command: Command{Action: "create_draft", ProjectID: "recover", OperationID: "recover-draft"}, Draft: draft()}
	if _, e := s.BuiltinInvoke(ctx, draftIn); e != nil {
		t.Fatal(e)
	}
	in := Invocation{Actor: Actor{Token: "owner-token"}, Command: Command{Action: "publish", ProjectID: "recover", OperationID: "recover-publish", Confirmation: "browser-confirmed"}}
	s.Store = deferredCommitFailure{p}
	if _, e := s.BuiltinInvoke(ctx, in); e == nil {
		t.Fatal("deferred LOCAL COMMIT unexpectedly succeeded")
	}
	if a.calls != 1 {
		t.Fatal("Core execution did not precede local COMMIT")
	}
	if e := p.Within(ctx, func(tx Tx) error {
		var row Project
		if e := tx.Get("projects", "recover", &row); e != nil {
			return e
		}
		if row.Status != "draft" {
			return ErrConflict
		}
		var op operation
		if e := tx.Get("operations", "recover-publish", &op); e != ErrNotFound {
			return ErrConflict
		}
		return nil
	}); e != nil {
		t.Fatal("failed commit retained projection", e)
	}
	reopened, e := NewPostgres(ctx, db, name)
	if e != nil {
		t.Fatal(e)
	}
	s = &Service{Store: reopened, Funds: a}
	out, e := s.RecoverOperation(ctx, "owner-token", in)
	if e != nil || out.Project.Status != "published" {
		t.Fatal(out, e)
	}
	if _, e = s.RecoverOperation(ctx, "owner-token", in); e != nil || a.calls != 1 || a.queries != 1 || a.previews != 0 {
		t.Fatal("recovery repeated Core write", e)
	}
	in = Invocation{Actor: Actor{Token: "worker-token"}, Command: Command{Action: "accept", ProjectID: "recover", ChallengeID: "recover-ch", OperationID: "recover-accept", GithubHandle: "contributor", Confirmation: "browser-confirmed"}}
	s.Store = deferredCommitFailure{reopened}
	if _, e = s.BuiltinInvoke(ctx, in); e == nil {
		t.Fatal("accept LOCAL COMMIT unexpectedly succeeded")
	}
	s.Store = reopened
	pause := Invocation{Actor: Actor{Token: "owner-token"}, Command: Command{Action: "pause", ProjectID: "recover", OperationID: "recover-pause", Confirmation: "browser-confirmed"}}
	if _, e = s.BuiltinInvoke(ctx, pause); e != nil {
		t.Fatal(e)
	}
	altered := in
	altered.Command.GithubHandle = "substituted"
	if _, e = s.RecoverOperation(ctx, "worker-token", altered); e != ErrConflict {
		t.Fatal("accepted substituted original command", e)
	}
	out, e = s.RecoverOperation(ctx, "worker-token", in)
	if e != nil || out.Challenge == nil || out.Challenge.GithubHandle != "contributor" {
		t.Fatal(out, e)
	}
	if e = reopened.Within(ctx, func(tx Tx) error {
		var ch Challenge
		if e := tx.Get("challenges", "recover-ch", &ch); e != nil {
			return e
		}
		if ch.Status != "accepted" || ch.ParticipantUserID != "worker" {
			return ErrConflict
		}
		var op operation
		return tx.Get("operations", "recover-accept", &op)
	}); e != nil {
		t.Fatal(e)
	}
	if a.calls != 3 || a.previews != 0 {
		t.Fatal("recovery issued financial action")
	}
	if e = reopened.Within(ctx, func(tx Tx) error {
		var p Project
		if e := tx.Get("projects", "recover", &p); e != nil {
			return e
		}
		if p.Status != "paused" || p.CoreRevision != 3 || p.EscrowCredit != 970 || p.PublishedAt != 100 {
			return ErrConflict
		}
		return nil
	}); e != nil {
		t.Fatal("older accept reverted newer project", e)
	}
	submit := Invocation{Actor: Actor{Token: "worker-token"}, Command: Command{Action: "submit", ProjectID: "recover", ChallengeID: "recover-ch", OperationID: "recover-submit", PullRequestURL: "https://github.com/owner/repo/pull/42", Confirmation: "browser-confirmed"}}
	if _, e = s.BuiltinInvoke(ctx, submit); e != nil {
		t.Fatal(e)
	}
	if _, e = db.ExecContext(ctx, `DELETE FROM lmm_bounties.records WHERE bucket='operations' AND id='recover-accept'`); e != nil {
		t.Fatal(e)
	}
	out, e = s.RecoverOperation(ctx, "worker-token", in)
	if e != nil || out.Challenge.Status != "submitted" || out.Challenge.CoreRevision != 4 || out.Challenge.PullRequestURL != "https://github.com/owner/repo/pull/42" {
		t.Fatal("older accept reverted newer challenge", out, e)
	}
	if a.calls != 4 {
		t.Fatal("old receipt recovery repeated action")
	}

}

func testPostgresRESTRecordsAndHTTP(t *testing.T, p *Postgres) {
	t.Helper()
	s, a, h := webSetup(t)
	s.Store = p
	ctx := context.Background()
	payload := `{"title":"REST integration","description":"a sufficiently detailed description","rules":"a sufficiently detailed set of rules","repository_url":"https://github.com/owner/repo","reward_quota":100,"reward_slots":2}`
	w := webRequest(h, "POST", "/api/open-source-bounties", "owner-token", payload, "rest-pg-draft")
	if w.Code != 200 {
		t.Fatal("REST draft", w.Code, w.Body.String())
	}
	id := dataMap(t, w)["id"].(string)
	w = webRequest(h, "POST", "/api/open-source-bounties/projects/"+id+"/publish", "owner-token", "", "rest-pg-publish")
	if w.Code != 202 {
		t.Fatal(w.Code, w.Body.String())
	}
	op := dataMap(t, w)["operation_id"].(string)
	a.approved[op] = true
	w = webRequest(h, "POST", "/api/open-source-bounties/operations/"+op+"/apply", "owner-token", "", "")
	if w.Code != 200 {
		t.Fatal("REST publish apply", w.Code, w.Body.String())
	}
	w = webRequest(h, "GET", "/api/open-source-bounties/projects/"+id, "", "", "")
	if w.Code != 200 {
		t.Fatal(w.Code, w.Body.String())
	}
	detail := dataMap(t, w)
	if len(detail["challenges"].([]any)) != 0 || len(detail["ledger"].([]any)) != 0 {
		t.Fatal("public detail private rows exposed")
	}
	w = webRequest(h, "GET", "/api/open-source-bounties/projects/"+id, "owner-token", "", "")
	if w.Code != 200 {
		t.Fatal(w.Code, w.Body.String())
	}
	if len(dataMap(t, w)["ledger"].([]any)) != 2 {
		t.Fatal("missing real receipt ledger projection", w.Body.String())
	}
	w = webRequest(h, "PUT", "/api/open-source-bounties/projects/"+id, "owner-token", `{"title":"Changed content","description":"a sufficiently detailed updated description","rules":"a sufficiently detailed updated set of rules"}`, "")
	if w.Code != 200 || dataMap(t, w)["reward_quota"] != float64(100) {
		t.Fatal("content altered economics", w.Code, w.Body.String())
	}
	var project Project
	s.Store.Within(ctx, func(tx Tx) error { return tx.Get("projects", id, &project) })
	ch := Challenge{ID: "rest-pg-ch", ProjectID: id, ParticipantUserID: "worker", OwnerUserID: "owner"}
	if e := s.Store.Within(ctx, func(tx Tx) error {
		if e := tx.Put("challenges", ch.ID, ch); e != nil {
			return e
		}
		return s.projectNotification(tx, Actor{UserID: "owner"}, Command{Action: "tip", OperationID: "rest-pg-tip", ProjectID: id, ChallengeID: ch.ID, ReviewNote: "thanks"}, Receipt{Credit: 7, JournalIDs: []int64{99}}, project, ch, 123)
	}); e != nil {
		t.Fatal(e)
	}
	out, e := s.Records(ctx, "worker-token", RecordRequest{Action: "tips_received"})
	if e != nil || len(out.Items) != 1 {
		t.Fatal(out, e)
	}
	if _, e = s.Records(ctx, "worker-token", RecordRequest{Action: "notifications_read"}); e != nil {
		t.Fatal(e)
	}
	out, e = s.Records(ctx, "worker-token", RecordRequest{Action: "thank_tip", NotificationID: "rest-pg-tip"})
	if e != nil || out.Notification.ThankedAt == 0 {
		t.Fatal(out, e)
	}
	if e = s.Store.Within(ctx, func(tx Tx) error {
		v := tx.(ViewTx)
		sum, e := v.Sum("notifications", Query{ProjectID: id, ChallengeID: ch.ID, TipsOnly: true})
		if e != nil || sum != 7 {
			t.Fatal("native SQL tip sum", sum, e)
		}
		hearts, e := v.Thanks("owner")
		if e != nil || hearts != 1 {
			t.Fatal("native SQL thanks", hearts, e)
		}
		project.Status = "closed"
		return tx.Put("projects", id, project)
	}); e != nil {
		t.Fatal(e)
	}
	if _, e = s.Records(ctx, "owner-token", RecordRequest{Action: "archive", ProjectID: id}); e != nil {
		t.Fatal(e)
	}
	out, e = s.Records(ctx, "owner-token", RecordRequest{Action: "mine", Archived: true})
	if e != nil || len(out.Items) != 1 {
		t.Fatal("SQL archive filter", out, e)
	}
}
