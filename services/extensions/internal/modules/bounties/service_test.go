package bounties

import (
	"context"
	"encoding/json"
	"sort"
	"strings"
	"sync"
	"testing"
	"time"
)

type memory struct {
	mu      sync.Mutex
	records map[string]json.RawMessage
}
type memoryTx struct{ records map[string]json.RawMessage }

func (m *memory) Within(_ context.Context, fn func(Tx) error) error {
	m.mu.Lock()
	defer m.mu.Unlock()
	next := map[string]json.RawMessage{}
	for k, v := range m.records {
		next[k] = append(json.RawMessage(nil), v...)
	}
	if e := fn(&memoryTx{next}); e != nil {
		return e
	}
	m.records = next
	return nil
}
func (m *memoryTx) Get(k, id string, out any) error {
	b, ok := m.records[k+":"+id]
	if !ok {
		return ErrNotFound
	}
	return json.Unmarshal(b, out)
}
func (m *memoryTx) Put(k, id string, v any) error {
	b, e := json.Marshal(v)
	m.records[k+":"+id] = b
	return e
}
func (m *memoryTx) List(k string, q Query) ([]json.RawMessage, error) {
	type item struct {
		id        string
		published int64
		raw       json.RawMessage
	}
	items := []item{}
	for id, b := range m.records {
		if !strings.HasPrefix(id, k+":") {
			continue
		}
		var v map[string]any
		json.Unmarshal(b, &v)
		if q.ProjectID != "" && v["project_id"] != q.ProjectID {
			continue
		}
		if q.ChallengeID != "" && v["challenge_id"] != q.ChallengeID {
			continue
		}
		if q.RecipientUserID != "" && v["participant_user_id"] != q.RecipientUserID {
			continue
		}
		if q.TipsOnly && v["kind"] != "tip_transfer" {
			continue
		}
		archived := false
		if f, ok := v["archived_at"].(float64); ok {
			archived = f > 0
		}
		if q.Archived != nil && archived != *q.Archived {
			continue
		}
		if q.Public && archived {
			continue
		}
		if q.Status != "" && v["status"] != q.Status {
			continue
		}
		if v["status"] == "deleted" {
			continue
		}
		if q.OwnerUserID != "" && v["owner_user_id"] != q.OwnerUserID {
			continue
		}
		if q.ParticipantUserID != "" && v["participant_user_id"] != q.ParticipantUserID {
			continue
		}
		if q.PartyUserID != "" && v["owner_user_id"] != q.PartyUserID && v["participant_user_id"] != q.PartyUserID {
			continue
		}
		if q.Public && v["status"] != "published" && v["status"] != "paused" {
			continue
		}
		var at int64
		if f, ok := v["published_at"].(float64); ok {
			at = int64(f)
		}
		items = append(items, item{strings.TrimPrefix(id, k+":"), at, b})
	}
	sort.Slice(items, func(i, j int) bool {
		if q.Public {
			if items[i].published != items[j].published {
				return items[i].published > items[j].published
			}
			return items[i].id > items[j].id
		}
		return items[i].id < items[j].id
	})
	out := []json.RawMessage{}
	skip := q.Offset
	cursor := q.After == ""
	for _, item := range items {
		if !cursor {
			if item.id == q.After {
				cursor = true
			}
			continue
		}
		if skip > 0 {
			skip--
			continue
		}
		if len(out) == q.Limit {
			break
		}
		out = append(out, item.raw)
	}
	return out, nil
}

type authority struct {
	calls int
	seen  Command
	wrong bool
	fail  bool
}

func (a *authority) Authorize(_ context.Context, token string) (Actor, error) {
	switch token {
	case "owner-token":
		return Actor{UserID: "owner"}, nil
	case "worker-token":
		return Actor{UserID: "worker"}, nil
	case "admin-token":
		return Actor{UserID: "admin", Admin: true}, nil
	}
	return Actor{}, ErrForbidden
}
func (a *authority) Preview(_ context.Context, _ string, c Command) (Receipt, error) {
	return Receipt{OperationID: c.OperationID, Confirmation: "core-confirmation"}, nil
}
func (a *authority) Execute(ctx context.Context, token string, c Command) (Receipt, error) {
	a.calls++
	a.seen = c
	if a.fail {
		return Receipt{}, ErrUnavailable
	}
	actor, _ := a.Authorize(ctx, token)
	state := map[string]string{"publish": "published", "pause": "paused", "resume": "published", "accept": "accepted", "submit": "submitted", "approve": "approved", "open_dispute": "open", "resolve_dispute": "resolved_denied"}[c.Action]
	if state == "" {
		state = "approved"
	}
	r := Receipt{OperationID: c.OperationID, ProjectID: c.ProjectID, ChallengeID: c.ChallengeID, DisputeID: c.DisputeID, ActorUserID: actor.UserID, State: state}
	if a.wrong {
		r.ActorUserID = "impostor"
	}
	return r, nil
}
func setup(t *testing.T) (*Service, *authority) {
	t.Helper()
	a := &authority{}
	return &Service{Store: &memory{records: map[string]json.RawMessage{}}, Funds: a}, a
}
func draft() Draft {
	return Draft{Title: "Test bounty", Description: strings.Repeat("x", 20), Rules: strings.Repeat("r", 20), RepositoryURL: "https://github.com/owner/repo", RewardCredit: 100, RewardSlots: 2}
}
func invoke(t *testing.T, s *Service, token, action, op string) (Result, error) {
	t.Helper()
	return s.BuiltinInvoke(context.Background(), Invocation{Actor: Actor{Token: token}, Command: Command{Action: action, OperationID: op, ProjectID: "p"}, Draft: draft()})
}
func TestDraftOwnershipComesFromLiveSession(t *testing.T) {
	s, _ := setup(t)
	_, e := s.BuiltinInvoke(context.Background(), Invocation{Actor: Actor{Token: "owner-token", UserID: "attacker", Admin: true}, Command: Command{Action: "create_draft", OperationID: "create", ProjectID: "p"}, Draft: draft()})
	if e != nil {
		t.Fatal(e)
	}
	out, e := invoke(t, s, "owner-token", "get", "")
	if e != nil || out.Project.OwnerUserID != "owner" {
		t.Fatalf("%+v %v", out, e)
	}
	if _, e = invoke(t, s, "worker-token", "get", ""); e != ErrForbidden {
		t.Fatalf("private draft leaked: %v", e)
	}
	if _, e = invoke(t, s, "worker-token", "delete_draft", "delete"); e != ErrForbidden {
		t.Fatalf("%v", e)
	}
}
func TestPublishUsesFrozenDraftAndIdempotentReceipt(t *testing.T) {
	s, a := setup(t)
	invoke(t, s, "owner-token", "create_draft", "create")
	in := Invocation{Actor: Actor{Token: "owner-token"}, Command: Command{Action: "publish", ProjectID: "p", OperationID: "publish", RewardCredit: 999, Confirmation: "c"}}
	out, e := s.BuiltinInvoke(context.Background(), in)
	if e != nil || out.Project.Status != "published" || a.seen.RewardCredit != 100 {
		t.Fatalf("%+v %v %v", out, e, a.seen)
	}
	_, e = s.BuiltinInvoke(context.Background(), in)
	if e != nil || a.calls != 1 {
		t.Fatalf("retry %v calls %d", e, a.calls)
	}
	in.Command.Credit = 1
	if _, e = s.BuiltinInvoke(context.Background(), in); e != ErrConflict {
		t.Fatalf("operation substitution %v", e)
	}
}
func TestAuthorityFailureDoesNotAdvanceProjection(t *testing.T) {
	s, a := setup(t)
	invoke(t, s, "owner-token", "create_draft", "create")
	a.fail = true
	if _, e := invoke(t, s, "owner-token", "publish", "publish"); e != ErrUnavailable {
		t.Fatal(e)
	}
	a.fail = false
	a.wrong = true
	if _, e := invoke(t, s, "owner-token", "publish", "publish"); e != ErrConflict {
		t.Fatal(e)
	}
	out, _ := invoke(t, s, "owner-token", "get", "")
	if out.Project.Status != "draft" {
		t.Fatal(out.Project.Status)
	}
}
func TestCrossRepositoryEvidenceAndOwnerSelfAccept(t *testing.T) {
	s, _ := setup(t)
	invoke(t, s, "owner-token", "create_draft", "create")
	invoke(t, s, "owner-token", "publish", "publish")
	in := Invocation{Actor: Actor{Token: "owner-token"}, Command: Command{Action: "accept", ProjectID: "p", ChallengeID: "ch", OperationID: "accept", GithubHandle: "dev"}}
	if _, e := s.BuiltinInvoke(context.Background(), in); e != ErrForbidden {
		t.Fatal(e)
	}
	in.Actor.Token = "worker-token"
	if _, e := s.BuiltinInvoke(context.Background(), in); e != nil {
		t.Fatal(e)
	}
	in.Command = Command{Action: "submit", ProjectID: "p", ChallengeID: "ch", OperationID: "submit", PullRequestURL: "https://github.com/other/repo/pull/1"}
	if _, e := s.BuiltinInvoke(context.Background(), in); e != ErrInvalid {
		t.Fatal(e)
	}
	in.Command.PullRequestURL = "https://github.com/owner/repo/pull/1"
	if _, e := s.BuiltinInvoke(context.Background(), in); e != nil {
		t.Fatal(e)
	}
}
func TestPreviewDoesNotMutateAndTokenNeverSerializes(t *testing.T) {
	s, a := setup(t)
	invoke(t, s, "owner-token", "create_draft", "create")
	out, e := s.BuiltinInvoke(context.Background(), Invocation{Actor: Actor{Token: "owner-token"}, Command: Command{Action: "publish", ProjectID: "p", OperationID: "pub"}, Preview: true})
	if e != nil || out.Receipt.Confirmation == "" || a.calls != 0 {
		t.Fatalf("%+v %v", out, e)
	}
	b, _ := json.Marshal(Actor{Token: "private"})
	if strings.Contains(string(b), "private") {
		t.Fatal("token serialized")
	}
}
func TestDraftValidation(t *testing.T) {
	for _, raw := range []string{"http://github.com/a/b", "https://github.com/a/b?token=x", "https://github.com/a/b/c", "https://user@github.com/a/b"} {
		if _, e := repository(raw); e != ErrInvalid {
			t.Fatal(raw)
		}
	}
	d := draft()
	d.RewardCredit = 1 << 62
	d.RewardSlots = 4
	if _, e := normalize(d); e != ErrInvalid {
		t.Fatal(e)
	}
}

func TestChallengeAndDisputeOnlyCallsResolvePersistedProject(t *testing.T) {
	s, _ := setup(t)
	invoke(t, s, "owner-token", "create_draft", "create")
	invoke(t, s, "owner-token", "publish", "pub")
	ctx := context.Background()
	in := Invocation{Actor: Actor{Token: "worker-token"}, Command: Command{Action: "accept", OperationID: "accept", ProjectID: "p", ChallengeID: "ch", GithubHandle: "dev"}}
	if _, e := s.BuiltinInvoke(ctx, in); e != nil {
		t.Fatal(e)
	}
	in.Command = Command{Action: "submit", OperationID: "submit", ChallengeID: "ch", PullRequestURL: "https://github.com/owner/repo/pull/1"}
	if _, e := s.BuiltinInvoke(ctx, in); e != nil {
		t.Fatal(e)
	}
	in.Command = Command{Action: "open_dispute", OperationID: "open", ChallengeID: "ch", DisputeID: "d", Reason: "other", Statement: strings.Repeat("s", 20)}
	if _, e := s.BuiltinInvoke(ctx, in); e != nil {
		t.Fatal(e)
	}
	in.Actor.Token = "admin-token"
	in.Command = Command{Action: "resolve_dispute", OperationID: "resolve", DisputeID: "d", ResolveAction: "deny", Resolution: strings.Repeat("r", 20)}
	out, e := s.BuiltinInvoke(ctx, in)
	if e != nil || out.Receipt.ProjectID != "p" || out.Receipt.ChallengeID != "ch" {
		t.Fatalf("%+v %v", out, e)
	}
	in.Command = Command{Action: "approve", OperationID: "substitute", ProjectID: "other", ChallengeID: "ch"}
	if _, e := s.BuiltinInvoke(ctx, in); e != ErrConflict {
		t.Fatal(e)
	}
}
func TestBoardSecondPageUsesPublishedTimeNotRandomID(t *testing.T) {
	s, _ := setup(t)
	ctx := context.Background()
	for n, id := range []string{"zzz", "aaa", "mmm"} {
		s.Now = func() time.Time { return time.Unix(int64(100+n), 0) }
		in := Invocation{Actor: Actor{Token: "owner-token"}, Draft: draft(), Command: Command{Action: "create_draft", OperationID: "draft-" + id, ProjectID: id}}
		if _, e := s.BuiltinInvoke(ctx, in); e != nil {
			t.Fatal(e)
		}
		in.Command.Action = "publish"
		in.Command.OperationID = "publish-" + id
		if _, e := s.BuiltinInvoke(ctx, in); e != nil {
			t.Fatal(e)
		}
	}
	out, e := s.BuiltinInvoke(ctx, Invocation{Actor: Actor{Token: "worker-token"}, Command: Command{Action: "list"}, Page: 2, PageSize: 1})
	if e != nil || len(out.Items) != 1 {
		t.Fatalf("%+v %v", out, e)
	}
	var p Project
	json.Unmarshal(out.Items[0], &p)
	if p.ID != "aaa" || p.PublishedAt != 101 {
		t.Fatalf("wrong second page %+v", p)
	}
	out, e = s.BuiltinInvoke(ctx, Invocation{Actor: Actor{Token: "owner-token"}, Command: Command{Action: "list_owned"}, Page: 2, PageSize: 1})
	if e != nil || len(out.Items) != 1 {
		t.Fatalf("%+v %v", out, e)
	}
}
func TestDisputeListAllowsHundredAndOrdinaryListRejectsIt(t *testing.T) {
	s, _ := setup(t)
	if _, e := s.BuiltinInvoke(context.Background(), Invocation{Actor: Actor{Token: "owner-token"}, Command: Command{Action: "list_disputes"}, Limit: 100}); e != nil {
		t.Fatal(e)
	}
	if _, e := s.BuiltinInvoke(context.Background(), Invocation{Actor: Actor{Token: "owner-token"}, Command: Command{Action: "list"}, Limit: 100}); e != ErrInvalid {
		t.Fatal(e)
	}
}

func TestDisputeStatusFilterRunsBeforePagination(t *testing.T) {
	s, _ := setup(t)
	ctx := context.Background()
	s.Store.Within(ctx, func(tx Tx) error {
		for _, d := range []Dispute{{ID: "a", Status: "open", OwnerUserID: "owner"}, {ID: "b", Status: "resolved_paid", OwnerUserID: "owner"}, {ID: "c", Status: "resolved_paid", OwnerUserID: "owner"}} {
			if e := tx.Put("disputes", d.ID, d); e != nil {
				return e
			}
		}
		return nil
	})
	in := Invocation{Actor: Actor{Token: "owner-token"}, Command: Command{Action: "list_disputes"}, Status: " resolved_paid ", Page: 2, PageSize: 1}
	out, e := s.BuiltinInvoke(ctx, in)
	if e != nil || len(out.Items) != 1 {
		t.Fatalf("%+v %v", out, e)
	}
	var d Dispute
	json.Unmarshal(out.Items[0], &d)
	if d.ID != "c" || d.Status != "resolved_paid" {
		t.Fatalf("%+v", d)
	}
	in.Status = "closed"
	if _, e = s.BuiltinInvoke(ctx, in); e != ErrInvalid {
		t.Fatal(e)
	}
}
func TestTipNoteBoundedAndRatingFieldsPreserved(t *testing.T) {
	s, a := setup(t)
	invoke(t, s, "owner-token", "create_draft", "create")
	invoke(t, s, "owner-token", "publish", "publish")
	in := Invocation{Actor: Actor{Token: "worker-token"}, Command: Command{Action: "accept", OperationID: "accept", ProjectID: "p", ChallengeID: "ch", GithubHandle: "dev"}}
	if _, e := s.BuiltinInvoke(context.Background(), in); e != nil {
		t.Fatal(e)
	}
	in.Actor.Token = "owner-token"
	in.Command = Command{Action: "tip", OperationID: "tip", ChallengeID: "ch", Credit: 5, ReviewNote: strings.Repeat("x", 501)}
	if _, e := s.BuiltinInvoke(context.Background(), in); e != ErrInvalid {
		t.Fatal(e)
	}
	in.Command.ReviewNote = " thank you "
	if _, e := s.BuiltinInvoke(context.Background(), in); e != nil || a.seen.ReviewNote != "thank you" {
		t.Fatal(e, a.seen.ReviewNote)
	}
	in.Actor.Token = "worker-token"
	in.Command = Command{Action: "rate_owner", OperationID: "rate", ChallengeID: "ch", RatingScore: 4, RatingComment: "clear requirements"}
	out, e := s.BuiltinInvoke(context.Background(), in)
	if e != nil || out.Challenge.ContributorRatingScore != 4 || out.Challenge.ContributorRatingComment != "clear requirements" {
		t.Fatalf("%+v %v", out, e)
	}
}

func (m *memoryTx) All(k string, q Query) ([]json.RawMessage, error) {
	q.Limit = int(^uint(0) >> 1)
	q.Offset = 0
	q.After = ""
	return m.List(k, q)
}
func (m *memoryTx) Count(k string, q Query) (int64, error) {
	rows, e := m.All(k, q)
	return int64(len(rows)), e
}
func (m *memoryTx) Sum(k string, q Query) (int64, error) {
	rows, e := m.All(k, q)
	var sum int64
	for _, raw := range rows {
		var n Notification
		json.Unmarshal(raw, &n)
		sum += n.Quota
	}
	return sum, e
}
func (m *memoryTx) Ratings(user string, owner bool) (int64, int64, error) {
	var sum, n int64
	for id, raw := range m.records {
		if !strings.HasPrefix(id, "challenges:") {
			continue
		}
		var ch Challenge
		json.Unmarshal(raw, &ch)
		score := 0
		if owner && ch.OwnerUserID == user {
			score = ch.ContributorRatingScore
		} else if !owner && ch.ParticipantUserID == user && !ch.OwnerRatingOverturned {
			score = ch.OwnerRatingScore
		}
		if score > 0 {
			sum += int64(score)
			n++
		}
	}
	return sum, n, nil
}
func (m *memoryTx) MarkRead(user string, tips bool, now int64) error {
	for id, raw := range m.records {
		if !strings.HasPrefix(id, "notifications:") {
			continue
		}
		var n Notification
		json.Unmarshal(raw, &n)
		if n.RecipientUserID == user && (!tips || n.Kind == "tip_transfer") && n.RecipientReadAt == 0 {
			n.RecipientReadAt = now
			if e := m.Put("notifications", n.ID, n); e != nil {
				return e
			}
		}
	}
	return nil
}

func (m *memoryTx) DistinctParticipants(project string) (int64, error) {
	rows, e := m.All("challenges", Query{ProjectID: project})
	users := map[string]bool{}
	for _, raw := range rows {
		var ch Challenge
		json.Unmarshal(raw, &ch)
		users[ch.ParticipantUserID] = true
	}
	return int64(len(users)), e
}
func (m *memoryTx) Thanks(user string) (int64, error) {
	rows, e := m.All("notifications", Query{})
	var n int64
	for _, raw := range rows {
		var v Notification
		json.Unmarshal(raw, &v)
		if v.SenderUserID == user && v.Kind == "tip_transfer" && v.ThankedAt > 0 {
			n++
		}
	}
	return n, e
}

func (m *memoryTx) Appeals(project string, now int64) (int64, int64, error) {
	rows, e := m.All("challenges", Query{ProjectID: project, Status: "rejected"})
	var n, ends int64
	for _, raw := range rows {
		var ch Challenge
		json.Unmarshal(raw, &ch)
		if ch.RejectedAt <= now-604800 {
			continue
		}
		ds, _ := m.All("disputes", Query{ChallengeID: ch.ID})
		resolved := false
		for _, dRaw := range ds {
			var d Dispute
			json.Unmarshal(dRaw, &d)
			if d.Status == "resolved_paid" || d.Status == "resolved_denied" {
				resolved = true
			}
		}
		if !resolved {
			n++
			if ch.RejectedAt+604800 > ends {
				ends = ch.RejectedAt + 604800
			}
		}
	}
	return n, ends, e
}
