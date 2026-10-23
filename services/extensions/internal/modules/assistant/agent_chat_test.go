package assistant

import (
	"context"
	"encoding/json"
	"errors"
	"io"
	"strings"
	"sync"
	"testing"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
)

// Memory transactions isolate consumer ordering; these are not PostgreSQL or
// independent Core/provider acceptance tests.
type agentChatJournalFixture struct {
	mu         sync.Mutex
	state      AgentLocalState
	failCommit bool
}

func (j *agentChatJournalFixture) WithinAgent(_ context.Context, owner, conversation int64, turn string, fn func(*AgentLocalState) error) error {
	j.mu.Lock()
	defer j.mu.Unlock()
	if owner != 1 || conversation != 10 {
		return access.ErrNotFound
	}
	raw, _ := json.Marshal(j.state)
	var next AgentLocalState
	_ = json.Unmarshal(raw, &next)
	if next.Turn != nil && next.Turn.ID != turn {
		return access.ErrConflict
	}
	if err := fn(&next); err != nil {
		return err
	}
	if next.Turn != nil && !validAgentStoredUpdate(j.state.Turn, next.Turn) {
		return access.ErrConflict
	}
	j.state = next
	if j.failCommit {
		j.failCommit = false
		return ErrDelegationUnknown
	}
	return nil
}

type agentChatExecutorFixture struct {
	mu                 sync.Mutex
	executes, inspects int
	failAfterEffect    bool
	firstUnknown       bool
	result             *AgentToolResult
	hold, entered      chan struct{}
}

func (e *agentChatExecutorFixture) Execute(_ context.Context, in AgentInvocation) (AgentToolResult, error) {
	e.mu.Lock()
	e.executes++
	e.mu.Unlock()
	if e.entered != nil {
		close(e.entered)
		<-e.hold
	}
	state := AgentToolSucceeded
	if e.firstUnknown {
		state = AgentToolUnknown
	}
	r, _ := AgentResult(in.Call, state, map[string]string{"value": string(state)}, "original-business")
	e.mu.Lock()
	e.result = &r
	fail := e.failAfterEffect
	e.failAfterEffect = false
	e.mu.Unlock()
	if fail {
		return AgentToolResult{}, io.EOF
	}
	return r, nil
}
func (e *agentChatExecutorFixture) Inspect(_ context.Context, in AgentInvocation) (AgentToolResult, error) {
	e.mu.Lock()
	defer e.mu.Unlock()
	e.inspects++
	if e.result != nil {
		return *e.result, nil
	}
	return AgentResult(in.Call, AgentToolPending, map[string]string{"state": "pending"}, "")
}

type agentChatPortFixture struct {
	mu                                 sync.Mutex
	snapshot                           AgentTurnSnapshot
	claim                              *AgentToolClaimInput
	starts, gets, claims, completes    int
	loseStart, loseClaim, loseComplete bool
	journal                            *agentChatJournalFixture
	receipts                           []AgentToolReceipt
	completions                        []AgentToolCompletion
}

func (p *agentChatPortFixture) StartAgentTurn(_ context.Context, _ string, in AgentTurnStart) (AgentTurnSnapshot, error) {
	p.mu.Lock()
	defer p.mu.Unlock()
	p.starts++
	p.journal.mu.Lock()
	persisted := p.journal.state.Turn != nil && p.journal.state.Turn.Phase == "start_inflight"
	p.journal.mu.Unlock()
	if !persisted {
		panic("start before durable intent")
	}
	p.snapshot.Key = in.Key
	p.snapshot.OriginalTextSHA256 = agentDigest([]byte(in.Text))
	if p.loseStart {
		p.loseStart = false
		return AgentTurnSnapshot{}, io.EOF
	}
	return p.snapshot, nil
}
func (p *agentChatPortFixture) GetAgentTurn(context.Context, string, AgentTurnKey) (AgentTurnSnapshot, error) {
	p.mu.Lock()
	defer p.mu.Unlock()
	p.gets++
	return p.snapshot, nil
}
func (p *agentChatPortFixture) ClaimAgentTool(_ context.Context, _ string, in AgentToolClaimInput) (AgentToolClaim, error) {
	p.mu.Lock()
	defer p.mu.Unlock()
	p.claims++
	p.journal.mu.Lock()
	tool := p.journal.state.Turn.Tools[in.OperationID]
	persisted := tool != nil && tool.ClaimKey == in.ClaimKey
	p.journal.mu.Unlock()
	if !persisted {
		panic("claim before durable intent")
	}
	fresh := p.claim == nil
	if p.claim != nil && *p.claim != in {
		return AgentToolClaim{}, access.ErrConflict
	}
	p.claim = &in
	if p.loseClaim {
		p.loseClaim = false
		return AgentToolClaim{}, io.EOF
	}
	claim := AgentToolClaim{Call: p.snapshot.PendingTools[0], ClaimKey: in.ClaimKey, MayExecute: fresh}
	if len(p.receipts) > 0 {
		receipt := p.receipts[len(p.receipts)-1]
		claim.AcceptedResult = &receipt
	}
	return claim, nil
}
func (p *agentChatPortFixture) CompleteAgentTool(_ context.Context, _ string, in AgentToolCompletion) (AgentToolReceipt, error) {
	p.mu.Lock()
	defer p.mu.Unlock()
	p.completes++
	if p.claim == nil || *p.claim != in.Claim {
		return AgentToolReceipt{}, access.ErrConflict
	}
	r := AgentToolReceipt{OperationID: in.Result.OperationID, ClaimKey: in.Claim.ClaimKey, State: AgentToolSucceeded, ResultSHA256: in.Result.ResultSHA256, DomainReceiptID: in.Result.DomainReceiptID}
	if in.Result.State == AgentToolUnknown {
		r.State = AgentToolUnknown
	} else {
		p.snapshot.Phase = AgentTurnCompleted
		p.snapshot.PendingTools = nil
		p.snapshot.FinalText = "Core final answer"
		p.snapshot.Revision++
	}
	p.receipts = append(p.receipts, r)
	p.completions = append(p.completions, in)
	if p.loseComplete {
		p.loseComplete = false
		return AgentToolReceipt{}, io.EOF
	}
	return r, nil
}
func agentChatFixture(t *testing.T) (*AgentChat, *agentChatJournalFixture, *agentChatPortFixture, *agentChatExecutorFixture, *chatHistoryFixture) {
	t.Helper()
	e := &agentChatExecutorFixture{}
	r, err := NewAgentRegistry([]AgentToolBinding{{Name: "tool", Description: "fixture", Effect: "server_guarded", Access: "user", Version: FixedMainToolVersion, PriceVersion: BuiltinToolPriceVersion, PriceSHA256: FreeAgentPriceSHA256(), Parameters: json.RawMessage(`{"type":"object","properties":{},"additionalProperties":false}`), Executor: e}})
	if err != nil {
		t.Fatal(err)
	}
	scope, _ := r.Scope("tool")
	coreID := "lmm_cv_" + strings.Repeat("a", 64)
	args := json.RawMessage(`{}`)
	call := AgentToolCall{OperationID: "original-operation", CoreConversationID: coreID, TurnID: "turn", ModelOperationID: "original-model", ProviderCallID: "original-provider", Scope: scope, Arguments: args, ArgumentsSHA256: agentDigest(args)}
	j := &agentChatJournalFixture{state: AgentLocalState{CanStart: true, Mapping: &DelegationConversationMapping{CreationKey: strings.Repeat("b", 32), CoreConversationID: coreID, Phase: "registered"}}}
	p := &agentChatPortFixture{journal: j, snapshot: AgentTurnSnapshot{Key: AgentTurnKey{coreID, "turn"}, OwnerUserID: 1, CoreOwnerAccountID: 123, Account: access.Account{Kind: "personal", ID: 1}, Revision: 1, MaxRounds: 8, Phase: AgentTurnAwaitingTools, BudgetCredits: 100, PendingTools: []AgentToolCall{call}}}
	a := &turnAuthorityFixture{}
	h := &chatHistoryFixture{}
	history, _ := NewHistory(h, a, []byte(strings.Repeat("k", 32)))
	chat, err := NewAgentChat(a, &PendingConversations{}, j, p, r, history)
	if err != nil {
		t.Fatal(err)
	}
	return chat, j, p, e, h
}

var agentRequest = PersonalChatRequest{ConversationID: 10, ClientTurnID: "turn", Message: "original question"}

func TestAgentChatCompletesOriginalToolThenCoreAnswer(t *testing.T) {
	c, j, p, e, h := agentChatFixture(t)
	reply, err := c.Chat(context.Background(), "session", agentRequest)
	if err != nil || reply.State != "settled" || reply.Text != "Core final answer" || e.executes != 1 || h.calls != 1 || p.starts != 1 {
		t.Fatal(reply, err, e.executes, h.calls)
	}
	if j.state.Turn.Tools["original-operation"].Receipt == nil || h.turn.UserContent != agentRequest.Message {
		t.Fatal("original binding lost")
	}
}
func TestAgentChatStartACKLossRecoveryOnlyInspects(t *testing.T) {
	c, j, p, e, h := agentChatFixture(t)
	p.loseStart = true
	if _, err := c.Chat(context.Background(), "session", agentRequest); err == nil {
		t.Fatal("expected ACK loss")
	}
	restart, _ := NewAgentChat(c.authority, c.pending, j, p, c.registry, c.history)
	reply, err := restart.Recover(context.Background(), "session", 10, "turn")
	if err != nil || reply.Text != "" || e.executes != 0 || e.inspects != 0 || p.claims != 0 || p.starts != 1 || h.calls != 0 {
		t.Fatal(reply, err, e.executes, e.inspects, p.starts)
	}
}
func TestAgentChatClaimACKLossNeverExecutes(t *testing.T) {
	c, _, p, e, h := agentChatFixture(t)
	p.loseClaim = true
	if _, err := c.Chat(context.Background(), "session", agentRequest); err == nil {
		t.Fatal("expected ACK loss")
	}
	reply, err := c.Chat(context.Background(), "session", agentRequest)
	if err != nil || reply.Text != "" || e.executes != 0 || e.inspects != 1 || p.claims != 2 || h.calls != 0 {
		t.Fatal(reply, err, e.executes, e.inspects)
	}
}
func TestAgentChatResultACKLossUsesFinalCoreReadback(t *testing.T) {
	c, j, p, e, h := agentChatFixture(t)
	p.loseComplete = true
	if _, err := c.Chat(context.Background(), "session", agentRequest); err == nil {
		t.Fatal("expected ACK loss")
	}
	restart, _ := NewAgentChat(c.authority, c.pending, j, p, c.registry, c.history)
	reply, err := restart.Recover(context.Background(), "session", 10, "turn")
	if err != nil || reply.Text != "Core final answer" || e.executes != 1 || p.starts != 1 || h.calls != 1 {
		t.Fatal(reply, err, e.executes, p.starts, h.calls)
	}
}
func TestAgentChatLocalCommitFailureSendsNoStartAndBindsInput(t *testing.T) {
	c, j, p, e, _ := agentChatFixture(t)
	j.failCommit = true
	if _, err := c.Chat(context.Background(), "session", agentRequest); err == nil || p.starts != 0 || e.executes != 0 {
		t.Fatal(err, p.starts, e.executes)
	}
	changed := agentRequest
	changed.Message = "changed"
	if _, err := c.Chat(context.Background(), "session", changed); !errors.Is(err, access.ErrConflict) {
		t.Fatal(err)
	}
}
func TestAgentChatRejectsOtherOwnerBeforeHistory(t *testing.T) {
	c, _, p, _, h := agentChatFixture(t)
	p.snapshot.OwnerUserID = 2
	if _, err := c.Chat(context.Background(), "session", agentRequest); err == nil || h.calls != 0 {
		t.Fatal(err, h.calls)
	}
}
func TestAgentChatConcurrentRetryCannotExecuteTwice(t *testing.T) {
	c, _, p, e, h := agentChatFixture(t)
	e.entered = make(chan struct{})
	e.hold = make(chan struct{})
	done := make(chan error, 1)
	go func() { _, err := c.Chat(context.Background(), "session", agentRequest); done <- err }()
	<-e.entered
	if _, err := c.Chat(context.Background(), "session", agentRequest); err != nil {
		t.Fatal(err)
	}
	close(e.hold)
	if err := <-done; err != nil {
		t.Fatal(err)
	}
	if e.executes != 1 || e.inspects != 1 || p.starts != 1 || h.calls != 1 {
		t.Fatal(e.executes, e.inspects, p.starts, h.calls)
	}
}

func TestAgentChatTitleRequiresOneAcceptedSuccessfulResult(t *testing.T) {
	c, _, _, _, _ := agentChatFixture(t)
	scope, _ := c.registry.Scope("tool")
	call := AgentToolCall{OperationID: "title", Scope: scope}
	call.Scope.Name = "set_conversation_title"
	result, _ := AgentResult(call, AgentToolSucceeded, map[string]any{"title": "Useful title", "applied": false, "new_conversation_only": true}, "title-receipt")
	tool := &AgentStoredTool{Call: call, Result: &result, Phase: "result_stored"}
	turn := AgentStoredTurn{Tools: map[string]*AgentStoredTool{"title": tool}}
	if title := agentCompletedTitle(turn); title != "" {
		t.Fatal("unaccepted title", title)
	}
	tool.Receipt = &AgentToolReceipt{}
	tool.Phase = "core_accepted"
	if title := agentCompletedTitle(turn); title != "Useful title" {
		t.Fatal(title)
	}
	other := *tool
	turn.Tools["other"] = &other
	if title := agentCompletedTitle(turn); title != "" {
		t.Fatal("ambiguous title", title)
	}
}

func TestAgentChatCommittedEffectWithLostACKOnlyInspectsOriginal(t *testing.T) {
	c, _, p, e, h := agentChatFixture(t)
	e.failAfterEffect = true
	if _, err := c.Chat(context.Background(), "session", agentRequest); err == nil {
		t.Fatal("expected domain ACK loss")
	}
	reply, err := c.Chat(context.Background(), "session", agentRequest)
	if err != nil || reply.Text != "Core final answer" || e.executes != 1 || e.inspects != 1 || p.starts != 1 || h.calls != 1 {
		t.Fatal(reply, err, e.executes, e.inspects, p.starts, h.calls)
	}
}

// UNKNOWN is an accepted uncertain result, not permission to run the effect
// again. Only inspection of the same original operation may resolve it.
func TestAgentChatUnknownReceiptConvergesByInspectOnOriginalClaim(t *testing.T) {
	c, j, p, e, h := agentChatFixture(t)
	e.firstUnknown = true
	reply, err := c.Chat(context.Background(), "session", agentRequest)
	if err != nil || reply.Text != "" || e.executes != 1 || e.inspects != 0 || p.completes != 1 || h.calls != 0 {
		t.Fatal(reply, err, e.executes, e.inspects, p.completes, h.calls)
	}
	originalClaim := *p.claim
	old := *j.state.Turn.Tools["original-operation"].Receipt
	if old.State != AgentToolUnknown {
		t.Fatal(old)
	}
	call := j.state.Turn.Tools["original-operation"].Call
	final, _ := AgentResult(call, AgentToolSucceeded, map[string]string{"value": "confirmed"}, "original-business")
	e.result = &final
	restart, _ := NewAgentChat(c.authority, c.pending, j, p, c.registry, c.history)
	reply, err = restart.Recover(context.Background(), "session", 10, "turn")
	if err != nil || reply.Text != "Core final answer" || e.executes != 1 || e.inspects != 1 || p.starts != 1 || p.claims != 2 || p.completes != 2 || h.calls != 1 {
		t.Fatal(reply, err, e.executes, e.inspects, p.starts, p.claims, p.completes, h.calls)
	}
	stored := j.state.Turn.Tools["original-operation"]
	if *p.claim != originalClaim || stored.ClaimKey != originalClaim.ClaimKey || !sameAgentJSON(stored.Call, call) || len(stored.ReceiptHistory) != 1 || !sameAgentJSON(stored.ReceiptHistory[0], old) || stored.Receipt == nil || stored.Receipt.State != AgentToolSucceeded || stored.Result.ResultSHA256 != final.ResultSHA256 || old.ResultSHA256 == stored.Result.ResultSHA256 {
		t.Fatal("original operation/claim or uncertain receipt history lost", stored)
	}
	for _, completion := range p.completions {
		if completion.Claim != originalClaim {
			t.Fatal("replacement claim", completion.Claim)
		}
	}
}

func TestAgentChatRepeatedUnknownInspectionDoesNotResubmitReceipt(t *testing.T) {
	c, j, p, e, h := agentChatFixture(t)
	e.firstUnknown = true
	if _, err := c.Chat(context.Background(), "session", agentRequest); err != nil {
		t.Fatal(err)
	}
	old := *j.state.Turn.Tools["original-operation"].Receipt
	reply, err := c.Recover(context.Background(), "session", 10, "turn")
	if err != nil || reply.Text != "" || e.executes != 1 || e.inspects != 1 || p.starts != 1 || p.claims != 2 || p.completes != 1 || h.calls != 0 {
		t.Fatal(reply, err, e.executes, e.inspects, p.starts, p.claims, p.completes, h.calls)
	}
	stored := j.state.Turn.Tools["original-operation"]
	if !sameAgentJSON(stored.Receipt, old) || len(stored.ReceiptHistory) != 0 {
		t.Fatal("uncertain receipt changed without terminal evidence", stored)
	}
	changed, _ := AgentResult(stored.Call, AgentToolUnknown, map[string]string{"state": "different uncertain payload"}, "original-business")
	e.result = &changed
	reply, err = c.Recover(context.Background(), "session", 10, "turn")
	stored = j.state.Turn.Tools["original-operation"]
	if err != nil || reply.Text != "" || e.executes != 1 || e.inspects != 2 || p.claims != 3 || p.completes != 1 || h.calls != 0 || stored.Result.ResultSHA256 != old.ResultSHA256 || !sameAgentJSON(stored.Receipt, old) || len(stored.ReceiptHistory) != 0 {
		t.Fatal("changed UNKNOWN payload replaced or resubmitted original uncertain receipt", reply, err, stored, p.completes)
	}
}
