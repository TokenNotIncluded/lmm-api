package assistant

import (
	"context"
	"errors"
	"strconv"
	"strings"
	"sync"
	"testing"

	"github.com/TokenNotIncluded/api.lmm.best/extensions/internal/modules/assistant/access"
)

// Ordering fixtures only; the real SQL implementation needs separate PG proof.
type pendingRepoFixture struct {
	mu            sync.Mutex
	rows          map[string]PendingConversation
	created       int
	unknownCommit bool
}

func pendingFixtureKey(owner int64, key string) string {
	return strconv.FormatInt(owner, 10) + ":" + key
}
func (r *pendingRepoFixture) CreatePending(_ context.Context, owner int64, key string, _ int64) (PendingConversation, bool, error) {
	r.mu.Lock()
	defer r.mu.Unlock()
	scope := pendingFixtureKey(owner, key)
	if p, ok := r.rows[scope]; ok {
		return p, false, nil
	}
	r.created++
	p := PendingConversation{ID: int64(r.created), CreationKey: key, Phase: "creation_inflight"}
	r.rows[scope] = p
	if r.unknownCommit {
		return PendingConversation{}, false, ErrDelegationUnknown
	}
	return p, true, nil
}
func (r *pendingRepoFixture) LookupPending(_ context.Context, owner int64, key string) (PendingConversation, error) {
	r.mu.Lock()
	defer r.mu.Unlock()
	p, ok := r.rows[pendingFixtureKey(owner, key)]
	if !ok {
		return p, access.ErrNotFound
	}
	return p, nil
}
func (r *pendingRepoFixture) BindPending(_ context.Context, owner int64, key, coreID string) (PendingConversation, error) {
	r.mu.Lock()
	defer r.mu.Unlock()
	scope := pendingFixtureKey(owner, key)
	p, ok := r.rows[scope]
	if !ok {
		return p, access.ErrNotFound
	}
	if p.CoreConversationID != "" && p.CoreConversationID != coreID {
		return p, access.ErrConflict
	}
	p.CoreConversationID, p.Phase = coreID, "registered"
	r.rows[scope] = p
	return p, nil
}

type pendingAuthorityFixture struct{}

func (pendingAuthorityFixture) Check(_ context.Context, credential string, _ access.Account, _ access.Permission) (access.Principal, error) {
	if credential != "session" && credential != "fresh-session" {
		return access.Principal{}, access.ErrUnauthorized
	}
	return access.Principal{UserID: 1, Account: access.Account{Kind: "personal", ID: 1}}, nil
}

type pendingPortFixture struct {
	*turnPortFixture
	credentialMu      sync.Mutex
	lookupCredentials []string
	beforeBegin       func(string)
}

func (p *pendingPortFixture) BeginConversation(ctx context.Context, credential, key string) (CoreConversationRegistration, error) {
	if p.beforeBegin != nil {
		p.beforeBegin(key)
	}
	return p.turnPortFixture.BeginConversation(ctx, credential, key)
}
func (p *pendingPortFixture) LookupConversation(ctx context.Context, credential, key string) (CoreConversationRegistration, error) {
	p.credentialMu.Lock()
	p.lookupCredentials = append(p.lookupCredentials, credential)
	p.credentialMu.Unlock()
	return p.turnPortFixture.LookupConversation(ctx, credential, key)
}
func pendingFixture(t *testing.T) (*PendingConversations, *pendingRepoFixture, *pendingPortFixture) {
	t.Helper()
	r := &pendingRepoFixture{rows: map[string]PendingConversation{}}
	p := &pendingPortFixture{turnPortFixture: &turnPortFixture{registrations: map[string]CoreConversationRegistration{}, operations: map[string]PersonalTextOperation{}}}
	s, err := NewPendingConversations(pendingAuthorityFixture{}, r, p)
	if err != nil {
		t.Fatal(err)
	}
	return s, r, p
}

func TestPendingConversationCommitsEmptyClaimBeforeCoreBegin(t *testing.T) {
	s, r, p := pendingFixture(t)
	key := strings.Repeat("a", 32)
	p.beforeBegin = func(key string) {
		local, err := r.LookupPending(context.Background(), 1, key)
		if err != nil || local.Phase != "creation_inflight" || local.CoreConversationID != "" {
			t.Error("Core Begin preceded local pending commit")
		}
	}
	local, err := s.Start(context.Background(), "session", key)
	if err != nil || local.Phase != "registered" || !coreConversationID(local.CoreConversationID) || r.created != 1 || p.beginCalls != 1 || p.issueCalls+p.reserveCalls != 0 {
		t.Fatal("pending creation created a fake completed turn or dispatched model work")
	}
	if _, err = s.Start(context.Background(), "session", key); err != nil || p.beginCalls != 1 || p.lookupConversationCalls != 1 || r.created != 1 {
		t.Fatal("same creation key repeated Begin or local creation")
	}
}

func TestPendingConversationUnknownLocalCommitNeverBegins(t *testing.T) {
	s, r, p := pendingFixture(t)
	key := strings.Repeat("b", 32)
	r.unknownCommit = true
	if _, err := s.Start(context.Background(), "session", key); !errors.Is(err, ErrDelegationUnknown) {
		t.Fatal(err)
	}
	if p.beginCalls+p.issueCalls+p.reserveCalls != 0 {
		t.Fatal("uncertain local commit sent a Core mutation")
	}
	if _, err := s.Recover(context.Background(), "session", key); !errors.Is(err, ErrDelegationUnknown) {
		t.Fatal("missing original receipt allowed creation")
	}
	if _, err := s.Start(context.Background(), "session", key); !errors.Is(err, ErrDelegationUnknown) {
		t.Fatal("existing pending claim was restarted")
	}
	if p.beginCalls != 0 || p.lookupConversationCalls != 2 || r.created != 1 {
		t.Fatal("recovery replaced the original creation claim")
	}
}

func TestPendingConversationLostCoreACKUsesOriginalKeyAndFreshSessionLookup(t *testing.T) {
	s, r, p := pendingFixture(t)
	key := strings.Repeat("c", 32)
	p.loseBeginACK = true
	local, err := s.Start(context.Background(), "session", key)
	if !errors.Is(err, ErrDelegationUnknown) || local.CreationKey != key || local.Phase != "creation_inflight" {
		t.Fatal("lost Core ACK was not pending/unknown")
	}
	reopened, err := NewPendingConversations(pendingAuthorityFixture{}, r, p)
	if err != nil {
		t.Fatal(err)
	}
	recovered, err := reopened.Recover(context.Background(), "fresh-session", key)
	if err != nil || recovered.ID != local.ID || recovered.Phase != "registered" || p.beginCalls != 1 || p.lookupConversationCalls != 1 || len(p.lookupCredentials) != 1 || p.lookupCredentials[0] != "fresh-session" {
		t.Fatal("recovery changed key, replayed Begin or forwarded the old credential")
	}
}

func TestPendingConversationConcurrentSameKeyHasOneBegin(t *testing.T) {
	s, r, p := pendingFixture(t)
	key := strings.Repeat("d", 32)
	var wg sync.WaitGroup
	for i := 0; i < 16; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			if _, err := s.Start(context.Background(), "session", key); err != nil && !errors.Is(err, ErrDelegationUnknown) {
				t.Error(err)
			}
		}()
	}
	wg.Wait()
	if p.beginCalls != 1 || r.created != 1 || p.issueCalls+p.reserveCalls != 0 {
		t.Fatal("concurrent creation repeated a mutation")
	}
}

func TestPendingConversationInvalidOrUnauthorizedHasNoLocalOrCoreEffect(t *testing.T) {
	s, r, p := pendingFixture(t)
	if _, err := s.Start(context.Background(), "session", "bad"); !errors.Is(err, access.ErrInvalid) {
		t.Fatal(err)
	}
	if _, err := s.Start(context.Background(), "revoked", strings.Repeat("e", 32)); !errors.Is(err, access.ErrUnauthorized) {
		t.Fatal(err)
	}
	if _, err := s.Recover(context.Background(), "session", strings.Repeat("e", 32)); !errors.Is(err, ErrDelegationUnknown) {
		t.Fatal(err)
	}
	if r.created != 0 || p.beginCalls+p.lookupConversationCalls+p.issueCalls+p.reserveCalls != 0 {
		t.Fatal("invalid/missing request mutated state")
	}
}
