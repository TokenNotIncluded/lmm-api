package assistant

import (
	"context"
	"errors"
	"testing"
	"time"

	"github.com/TokenNotIncluded/lmm-api/services/extensions/internal/modules/assistant/access"
)

// Memory fixtures test consumer control flow only, not worker or PostgreSQL
// acceptance. All transitions below are deterministic, without racing timers.
type resumeCaptureExecutor struct {
	*agentChatExecutorFixture
	executeInputs, inspectInputs []string
}

func (e *resumeCaptureExecutor) Execute(ctx context.Context, in AgentInvocation) (AgentToolResult, error) {
	e.executeInputs = append(e.executeInputs, in.OriginalUserText)
	return e.agentChatExecutorFixture.Execute(ctx, in)
}
func (e *resumeCaptureExecutor) Inspect(ctx context.Context, in AgentInvocation) (AgentToolResult, error) {
	e.inspectInputs = append(e.inspectInputs, in.OriginalUserText)
	return e.agentChatExecutorFixture.Inspect(ctx, in)
}
func captureResumeExecutor(c *AgentChat, e *agentChatExecutorFixture) *resumeCaptureExecutor {
	capture := &resumeCaptureExecutor{agentChatExecutorFixture: e}
	binding := c.registry.bindings["tool"]
	binding.Executor = capture
	c.registry.bindings["tool"] = binding
	return capture
}

func TestAgentResumeUsesStoredInputAndFreshPendingClaimWithoutRestart(t *testing.T) {
	c, j, p, e, h := agentChatFixture(t)
	capture := captureResumeExecutor(c, e)
	p.loseStart = true
	if _, err := c.Chat(context.Background(), "session", agentRequest); err == nil {
		t.Fatal("expected lost start acknowledgement")
	}
	// GET recovery has no authority to create a fresh tool claim.
	reply, err := c.Recover(context.Background(), "session", 10, "turn")
	if err != nil || reply.Text != "" || p.claims != 0 || e.executes != 0 {
		t.Fatal(reply, err, p.claims, e.executes)
	}
	restart, _ := NewAgentChat(c.authority, c.pending, j, p, c.registry, c.history)
	reply, err = restart.Resume(context.Background(), "session", 10, "turn")
	if err != nil || reply.Text != "Core final answer" || p.starts != 1 || p.claims != 1 || e.executes != 1 || h.calls != 1 {
		t.Fatal(reply, err, p.starts, p.claims, e.executes, h.calls)
	}
	if len(capture.executeInputs) != 1 || capture.executeInputs[0] != agentRequest.Message || h.turn.UserContent != agentRequest.Message {
		t.Fatal("resume did not use immutable original input", capture.executeInputs, h.turn)
	}
}

func TestAgentResumeOldClaimOnlyInspectsOriginalOperation(t *testing.T) {
	c, _, p, e, h := agentChatFixture(t)
	capture := captureResumeExecutor(c, e)
	p.loseClaim = true
	if _, err := c.Chat(context.Background(), "session", agentRequest); err == nil {
		t.Fatal("expected lost claim acknowledgement")
	}
	original := *p.claim
	reply, err := c.Resume(context.Background(), "session", 10, "turn")
	if err != nil || reply.Text != "" || p.starts != 1 || p.claims != 2 || e.executes != 0 || e.inspects != 1 || h.calls != 0 || *p.claim != original {
		t.Fatal(reply, err, p.starts, p.claims, e.executes, e.inspects)
	}
	if len(capture.inspectInputs) != 1 || capture.inspectInputs[0] != agentRequest.Message {
		t.Fatal("resume reconstructed a different intent", capture.inspectInputs)
	}
}

// Async worker view: Start returns MODEL_RUNNING; two Get calls publish the
// original pending tools. Completion initially leaves that same pending view,
// then a later Get publishes Core's final settled answer.
type asyncResumePort struct {
	*agentChatPortFixture
	pending                      AgentTurnSnapshot
	final                        *AgentTurnSnapshot
	beforeTools, afterCompletion int
	cancelAfterStart             context.CancelFunc
}

func (p *asyncResumePort) StartAgentTurn(ctx context.Context, credential string, start AgentTurnStart) (AgentTurnSnapshot, error) {
	snap, err := p.agentChatPortFixture.StartAgentTurn(ctx, credential, start)
	if err != nil {
		return snap, err
	}
	p.pending = snap
	p.pending.Revision++
	p.mu.Lock()
	p.snapshot.Phase = AgentTurnRunning
	p.snapshot.PendingTools = nil
	snap = p.snapshot
	p.mu.Unlock()
	if p.cancelAfterStart != nil {
		p.cancelAfterStart()
	}
	return snap, nil
}
func (p *asyncResumePort) GetAgentTurn(ctx context.Context, credential string, key AgentTurnKey) (AgentTurnSnapshot, error) {
	p.mu.Lock()
	if p.final == nil {
		p.beforeTools++
		if p.beforeTools >= 2 {
			p.snapshot = p.pending
		}
	} else {
		p.afterCompletion++
		if p.afterCompletion >= 3 {
			p.snapshot = *p.final
		}
	}
	p.mu.Unlock()
	return p.agentChatPortFixture.GetAgentTurn(ctx, credential, key)
}
func (p *asyncResumePort) CompleteAgentTool(ctx context.Context, credential string, in AgentToolCompletion) (AgentToolReceipt, error) {
	receipt, err := p.agentChatPortFixture.CompleteAgentTool(ctx, credential, in)
	if err != nil {
		return receipt, err
	}
	p.mu.Lock()
	final := p.snapshot
	p.final = &final
	p.snapshot = p.pending
	p.mu.Unlock()
	return receipt, nil
}
func TestAgentChatPollsAsyncWorkerThroughToolAndFinalInSingleRequest(t *testing.T) {
	c, _, p, e, h := agentChatFixture(t)
	async := &asyncResumePort{agentChatPortFixture: p}
	c.port = async
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()
	reply, err := c.Chat(ctx, "session", agentRequest)
	if err != nil || reply.Text != "Core final answer" || p.starts != 1 || p.claims != 1 || p.completes != 1 || e.executes != 1 || h.calls != 1 || async.beforeTools != 2 || async.afterCompletion != 3 {
		t.Fatal(reply, err, p.starts, p.claims, p.completes, e.executes, h.calls, async.beforeTools, async.afterCompletion)
	}
}
func TestAgentChatCanceledAsyncPollDoesNotStartAgain(t *testing.T) {
	c, _, p, e, h := agentChatFixture(t)
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	c.port = &asyncResumePort{agentChatPortFixture: p, cancelAfterStart: cancel}
	began := time.Now()
	reply, err := c.Chat(ctx, "session", agentRequest)
	if !errors.Is(err, ErrDelegationUnknown) || time.Since(began) > time.Second || reply.State != string(AgentTurnRunning) || p.starts != 1 || p.gets != 0 || p.claims != 0 || e.executes != 0 || h.calls != 0 {
		t.Fatal(reply, err, time.Since(began), p.starts, p.gets, p.claims, e.executes)
	}
}
func TestAgentResumeCannotInventMissingTurnOrBypassRevokedSession(t *testing.T) {
	c, _, p, e, h := agentChatFixture(t)
	if _, err := c.Resume(context.Background(), "session", 10, "turn"); !errors.Is(err, access.ErrNotFound) {
		t.Fatal(err)
	}
	c.authority.(*turnAuthorityFixture).revoked = true
	if _, err := c.Resume(context.Background(), "session", 10, "turn"); !errors.Is(err, access.ErrUnauthorized) {
		t.Fatal(err)
	}
	if p.starts != 0 || p.gets != 0 || p.claims != 0 || e.executes != 0 || h.calls != 0 {
		t.Fatal("resume created missing or unauthorized work")
	}
}
