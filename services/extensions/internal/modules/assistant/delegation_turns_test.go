package assistant

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"io"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/TokenNotIncluded/api.lmm.best/extensions/internal/modules/assistant/access"
)

// These fixtures isolate journal ordering and consumer recovery. They are not
// PostgreSQL, real Session authorization, process restart or funds acceptance.
type turnJournalFixture struct {
	mu             sync.Mutex
	state          DelegationLocalState
	failCommitOnce bool
	conversationID int64
	owner          int64
}

func (j *turnJournalFixture) Within(_ context.Context, owner, conversation int64, turn string, fn func(*DelegationLocalState) error) error {
	j.mu.Lock()
	defer j.mu.Unlock()
	if owner != j.owner || conversation != j.conversationID {
		return access.ErrNotFound
	}
	encoded, _ := json.Marshal(j.state)
	var state DelegationLocalState
	_ = json.Unmarshal(encoded, &state)
	if state.Turn != nil && state.Turn.ID != turn {
		return access.ErrConflict
	}
	if err := fn(&state); err != nil {
		return err
	}
	j.state = state
	if j.failCommitOnce {
		j.failCommitOnce = false
		return ErrDelegationUnknown
	}
	return nil
}

type turnAuthorityFixture struct{ revoked bool }

func (a *turnAuthorityFixture) Check(_ context.Context, credential string, _ access.Account, _ access.Permission) (access.Principal, error) {
	if a.revoked || credential != "session" {
		return access.Principal{}, access.ErrUnauthorized
	}
	return access.Principal{UserID: 1, Account: access.Account{Kind: "personal", ID: 1}}, nil
}

type turnPortFixture struct {
	mu                                                                                           sync.Mutex
	registrations                                                                                map[string]CoreConversationRegistration
	grant                                                                                        *PersonalTextGrant
	operations                                                                                   map[string]PersonalTextOperation
	beginCalls, lookupConversationCalls, issueCalls, lookupIssueCalls, reserveCalls, statusCalls int
	loseBeginACK, loseIssueACK, loseReserveACK                                                   bool
	beforeReserve                                                                                func(PersonalTextReservation)
}

func (p *turnPortFixture) BeginConversation(_ context.Context, _ string, key string) (CoreConversationRegistration, error) {
	p.mu.Lock()
	defer p.mu.Unlock()
	p.beginCalls++
	sum := sha256.Sum256([]byte(key))
	r := CoreConversationRegistration{CreationKey: key, ID: "lmm_cv_" + hex.EncodeToString(sum[:])}
	p.registrations[key] = r
	if p.loseBeginACK {
		return CoreConversationRegistration{}, io.EOF
	}
	return r, nil
}
func (p *turnPortFixture) LookupConversation(_ context.Context, _ string, key string) (CoreConversationRegistration, error) {
	p.mu.Lock()
	defer p.mu.Unlock()
	p.lookupConversationCalls++
	value, ok := p.registrations[key]
	if !ok {
		return value, access.ErrNotFound
	}
	return value, nil
}
func (p *turnPortFixture) Issue(_ context.Context, _ string, issue PersonalTextTurnKey) (PersonalTextGrant, error) {
	p.mu.Lock()
	defer p.mu.Unlock()
	p.issueCalls++
	grant := PersonalTextGrant{ID: "lmm_ad_" + strings.Repeat("a", 64), CoreConversationID: issue.CoreConversationID, TurnID: issue.TurnID, ExpiresAt: time.Now().Add(time.Minute).Unix(), MaxModelCalls: 4}
	p.grant = &grant
	if p.loseIssueACK {
		return PersonalTextGrant{}, io.EOF
	}
	return grant, nil
}
func (p *turnPortFixture) LookupIssue(_ context.Context, _ string, issue PersonalTextTurnKey) (PersonalTextGrant, error) {
	p.mu.Lock()
	defer p.mu.Unlock()
	p.lookupIssueCalls++
	if p.grant == nil || p.grant.CoreConversationID != issue.CoreConversationID || p.grant.TurnID != issue.TurnID {
		return PersonalTextGrant{}, access.ErrNotFound
	}
	return *p.grant, nil
}
func (p *turnPortFixture) Reserve(_ context.Context, _ string, invocation PersonalTextReservation) (PersonalTextOperation, error) {
	if p.beforeReserve != nil {
		p.beforeReserve(invocation)
	}
	p.mu.Lock()
	defer p.mu.Unlock()
	p.reserveCalls++
	outcome := PersonalTextOperation{Binding: invocation.Binding, RequestID: strings.Repeat("b", 64), Status: "reserved", ReservedCredits: 10}
	p.operations[invocation.Binding.OperationKey] = outcome
	if p.loseReserveACK {
		return PersonalTextOperation{}, io.EOF
	}
	return outcome, nil
}
func (p *turnPortFixture) Status(_ context.Context, _ string, binding PersonalTextBinding) (PersonalTextOperation, error) {
	p.mu.Lock()
	defer p.mu.Unlock()
	p.statusCalls++
	value, ok := p.operations[binding.OperationKey]
	if !ok {
		return PersonalTextOperation{}, access.ErrNotFound
	}
	return value, nil
}

func durableFixture(t *testing.T) (*DelegatedTurns, *turnJournalFixture, *turnPortFixture, *turnAuthorityFixture) {
	t.Helper()
	j := &turnJournalFixture{state: DelegationLocalState{CanStart: true}, owner: 1, conversationID: 10}
	p := &turnPortFixture{registrations: map[string]CoreConversationRegistration{}, operations: map[string]PersonalTextOperation{}}
	a := &turnAuthorityFixture{}
	s, err := NewDelegatedTurns(a, j, p)
	if err != nil {
		t.Fatal(err)
	}
	return s, j, p, a
}

func TestDurableTurnLostIssueACKUsesOnlyOriginalLookup(t *testing.T) {
	s, j, p, a := durableFixture(t)
	p.loseIssueACK = true
	request := ModelRequest{Input: "question"}
	if _, err := s.OpenTurn(context.Background(), "session", 10, "turn", request); !errors.Is(err, ErrDelegationUnknown) {
		t.Fatal("lost issue ACK was not unknown")
	}
	recreated, err := NewDelegatedTurns(a, j, p)
	if err != nil {
		t.Fatal(err)
	}
	grant, err := recreated.RecoverTurn(context.Background(), "session", 10, "turn")
	if err != nil || !delegationID(grant.ID) || p.beginCalls != 1 || p.issueCalls != 1 || p.lookupIssueCalls != 1 || p.reserveCalls != 0 {
		t.Fatal("recovery minted or reserved instead of looking up original issue")
	}
}

func TestDurableCreationLostACKDoesNotBeginOrIssueDuringRecovery(t *testing.T) {
	s, _, p, _ := durableFixture(t)
	p.loseBeginACK = true
	if _, err := s.OpenTurn(context.Background(), "session", 10, "turn", ModelRequest{Input: "question"}); !errors.Is(err, ErrDelegationUnknown) {
		t.Fatal(err)
	}
	if _, err := s.RecoverTurn(context.Background(), "session", 10, "turn"); !errors.Is(err, ErrDelegationUnknown) {
		t.Fatal("unissued grant must stay unknown")
	}
	if p.beginCalls != 1 || p.lookupConversationCalls != 1 || p.issueCalls != 0 || p.lookupIssueCalls != 1 {
		t.Fatal("lookup reissued conversation or grant")
	}
}

func TestDurableRoundPersistsBeforeReserveAndUnknownOnlyReadsStatus(t *testing.T) {
	s, j, p, a := durableFixture(t)
	request := ModelRequest{Input: "question"}
	if _, err := s.OpenTurn(context.Background(), "session", 10, "turn", request); err != nil {
		t.Fatal(err)
	}
	p.beforeReserve = func(in PersonalTextReservation) {
		j.mu.Lock()
		defer j.mu.Unlock()
		if j.state.Turn == nil || len(j.state.Turn.Rounds) != 1 || j.state.Turn.Rounds[0].Reservation != in || j.state.Turn.Rounds[0].Phase != "reserve_inflight" {
			t.Error("reserve occurred before exact original claim was persisted")
		}
	}
	p.loseReserveACK = true
	if _, err := s.ReserveRound(context.Background(), "session", 10, "turn", 0, request); !errors.Is(err, ErrDelegationUnknown) {
		t.Fatal(err)
	}
	recreated, err := NewDelegatedTurns(a, j, p)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := recreated.RecoverRound(context.Background(), "session", 10, "turn", 0); err != nil {
		t.Fatal(err)
	}
	if _, err := recreated.ReserveRound(context.Background(), "session", 10, "turn", 0, request); err != nil {
		t.Fatal(err)
	}
	if p.reserveCalls != 1 || p.statusCalls != 2 || p.issueCalls != 1 {
		t.Fatal("uncertain operation was reserved again")
	}
	if _, err := recreated.ReserveRound(context.Background(), "session", 10, "turn", 1, request); !errors.Is(err, access.ErrConflict) {
		t.Fatal("nonterminal predecessor allowed a new round")
	}
	if _, err := recreated.ReserveRound(context.Background(), "session", 10, "turn", 0, ModelRequest{Input: "changed"}); !errors.Is(err, access.ErrConflict) {
		t.Fatal("changed payload reused original operation")
	}
}

func TestDurableUnknownJournalCommitNeverCallsReserve(t *testing.T) {
	s, j, p, _ := durableFixture(t)
	request := ModelRequest{Input: "question"}
	if _, err := s.OpenTurn(context.Background(), "session", 10, "turn", request); err != nil {
		t.Fatal(err)
	}
	j.failCommitOnce = true
	if _, err := s.ReserveRound(context.Background(), "session", 10, "turn", 0, request); !errors.Is(err, ErrDelegationUnknown) {
		t.Fatal(err)
	}
	if _, err := s.RecoverRound(context.Background(), "session", 10, "turn", 0); !errors.Is(err, ErrDelegationUnknown) {
		t.Fatal("missing Core original should stay unknown")
	}
	if p.reserveCalls != 0 || p.statusCalls != 1 {
		t.Fatal("unknown local COMMIT authorized reserve")
	}
}

func TestDurableConcurrentOriginalRoundHasOneReserve(t *testing.T) {
	s, _, p, _ := durableFixture(t)
	request := ModelRequest{Input: "question"}
	if _, err := s.OpenTurn(context.Background(), "session", 10, "turn", request); err != nil {
		t.Fatal(err)
	}
	var wg sync.WaitGroup
	for i := 0; i < 16; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			if _, err := s.ReserveRound(context.Background(), "session", 10, "turn", 0, request); err != nil && !errors.Is(err, ErrDelegationUnknown) {
				t.Error(err)
			}
		}()
	}
	wg.Wait()
	if p.reserveCalls != 1 || p.issueCalls != 1 {
		t.Fatal("concurrent calls repeated original reservation or issue")
	}
}

func TestDurableUnsupportedForeignAndRevokedHaveNoPortEffect(t *testing.T) {
	s, j, p, a := durableFixture(t)
	if _, err := s.OpenTurn(context.Background(), "session", 10, "turn", ModelRequest{Input: "question", Tools: []Definition{{Summary: Summary{Name: "tool"}}}}); !errors.Is(err, ErrDelegationUnavailable) {
		t.Fatal(err)
	}
	j.owner = 2
	if _, err := s.OpenTurn(context.Background(), "session", 10, "turn", ModelRequest{Input: "question"}); !errors.Is(err, access.ErrNotFound) {
		t.Fatal(err)
	}
	j.owner = 1
	a.revoked = true
	if _, err := s.OpenTurn(context.Background(), "session", 10, "turn", ModelRequest{Input: "question"}); !errors.Is(err, access.ErrUnauthorized) {
		t.Fatal(err)
	}
	if p.beginCalls+p.issueCalls+p.reserveCalls != 0 || j.state.Turn != nil {
		t.Fatal("unsupported or unauthorized request mutated business state")
	}
}

func TestDurableTextDigestIsDistinctFromExactRawPayloadDigest(t *testing.T) {
	s, j, _, _ := durableFixture(t)
	request := ModelRequest{Input: "literal <&>\n文本"}
	if _, err := s.OpenTurn(context.Background(), "session", 10, "turn", request); err != nil {
		t.Fatal(err)
	}
	if _, err := s.ReserveRound(context.Background(), "session", 10, "turn", 0, request); err != nil {
		t.Fatal(err)
	}
	round := j.state.Turn.Rounds[0]
	raw, _ := json.Marshal(request)
	rawDigest, textDigest := sha256.Sum256(raw), sha256.Sum256([]byte(request.Input))
	if round.Invocation.RequestJSON != string(raw) || round.Invocation.PayloadSHA256 != hex.EncodeToString(rawDigest[:]) ||
		round.Reservation.Text != request.Input || round.Reservation.Binding.TextSHA256 != hex.EncodeToString(textDigest[:]) ||
		round.Invocation.PayloadSHA256 == round.Reservation.Binding.TextSHA256 {
		t.Fatal("raw ModelRequest digest was reused as the Core personal text digest")
	}
}

func TestDurableReceiptIdentityAndTerminalStateCannotBeRewritten(t *testing.T) {
	for _, change := range []string{"request_id", "reserved_credits", "terminal_regression", "terminal_charge"} {
		t.Run(change, func(t *testing.T) {
			s, j, p, _ := durableFixture(t)
			request := ModelRequest{Input: "question"}
			if _, err := s.OpenTurn(context.Background(), "session", 10, "turn", request); err != nil {
				t.Fatal(err)
			}
			if _, err := s.ReserveRound(context.Background(), "session", 10, "turn", 0, request); err != nil {
				t.Fatal(err)
			}
			before := j.state.Turn.Rounds[0].Outcome
			mutated := *before
			if strings.HasPrefix(change, "terminal_") {
				settled := int64(7)
				mutated.Status, mutated.SettledCredits = "settled", &settled
				p.operations["turn:0"] = mutated
				if _, err := s.RecoverRound(context.Background(), "session", 10, "turn", 0); err != nil {
					t.Fatal(err)
				}
				before = j.state.Turn.Rounds[0].Outcome
			}
			switch change {
			case "request_id":
				mutated.RequestID = strings.Repeat("c", 64)
			case "reserved_credits":
				mutated.ReservedCredits++
			case "terminal_regression":
				mutated.Status, mutated.SettledCredits = "unknown", nil
			case "terminal_charge":
				settled := int64(8)
				mutated.SettledCredits = &settled
			}
			p.operations["turn:0"] = mutated
			if _, err := s.RecoverRound(context.Background(), "session", 10, "turn", 0); !errors.Is(err, access.ErrConflict) {
				t.Fatal("Core operation identity or terminal receipt was rewritten", err)
			}
			if !validPersonalTextTransition(*before, *j.state.Turn.Rounds[0].Outcome) || p.reserveCalls != 1 {
				t.Fatal("failed lookup changed the original receipt or reserved again")
			}
		})
	}
}

func TestDurableJournalRefusesClaimAndPayloadRollback(t *testing.T) {
	s, j, _, _ := durableFixture(t)
	request := ModelRequest{Input: "question"}
	if _, err := s.OpenTurn(context.Background(), "session", 10, "turn", request); err != nil {
		t.Fatal(err)
	}
	if _, err := s.ReserveRound(context.Background(), "session", 10, "turn", 0, request); err != nil {
		t.Fatal(err)
	}
	before := j.state.Turn
	for _, change := range []string{"drop_round", "drop_outcome", "rewrite_text", "rewrite_claim", "drop_grant"} {
		t.Run(change, func(t *testing.T) {
			encoded, _ := json.Marshal(before)
			var after DelegationStoredTurn
			_ = json.Unmarshal(encoded, &after)
			switch change {
			case "drop_round":
				after.Rounds = nil
			case "drop_outcome":
				after.Rounds[0].Outcome, after.Rounds[0].Phase = nil, "reserve_inflight"
			case "rewrite_text":
				after.Rounds[0].Reservation.Text = "different"
			case "rewrite_claim":
				after.Rounds[0].Invocation.RequestJSON = "{}"
			case "drop_grant":
				after.Grant, after.Rounds, after.Phase = nil, nil, "created"
			}
			if validStoredDelegationUpdate(before, &after) {
				t.Fatal("journal accepted an original-operation rewrite")
			}
		})
	}
	claim := &DelegationStoredTurn{ID: "turn", InputHash: before.InputHash, Phase: "issue_inflight"}
	rollback := *claim
	rollback.Phase = "created"
	if validStoredDelegationUpdate(claim, &rollback) {
		t.Fatal("issue claim became eligible for a replacement Issue")
	}
}
