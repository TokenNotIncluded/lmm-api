package assistant

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"io"
	"strings"
	"testing"
	"time"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
)

// This fixture exercises the consumer contract only. It is not a live Core
// issuer, authenticated RPC, budget transaction or provider execution proof.
type delegationFixture struct {
	grant       DelegationGrant
	outcome     func(DelegationBinding) DelegationOutcome
	issueError  error
	invokeError error
	issues      int
	invokes     int
	statuses    int
}

func (f *delegationFixture) Issue(_ context.Context, _ string, _ DelegationIssue) (DelegationGrant, error) {
	f.issues++
	return f.grant, f.issueError
}
func (f *delegationFixture) Invoke(_ context.Context, _ string, request DelegationInvocation) (DelegationOutcome, error) {
	f.invokes++
	if f.invokeError != nil {
		return DelegationOutcome{}, f.invokeError
	}
	return f.outcome(request.DelegationBinding), nil
}
func (f *delegationFixture) Status(_ context.Context, _ string, request DelegationBinding) (DelegationOutcome, error) {
	f.statuses++
	return f.outcome(request), nil
}

func delegationTestClient(t *testing.T) (*DelegationClient, *delegationFixture) {
	t.Helper()
	f := &delegationFixture{grant: DelegationGrant{Version: 1, ID: "lmm_ad_" + strings.Repeat("a", 64), ConversationID: "conversation-1", TurnID: "turn-1", ExpiresAt: 1060, MaxModelCalls: 4}}
	f.outcome = func(binding DelegationBinding) DelegationOutcome {
		amount := "12"
		return DelegationOutcome{DelegationBinding: binding, Status: "settled", ChargedCredits: &amount, Reply: &ModelReply{Text: "reply", Calls: []ToolCall{}}}
	}
	c, err := NewDelegationClient(f)
	if err != nil {
		t.Fatal(err)
	}
	c.now = func() time.Time { return time.Unix(1000, 0) }
	return c, f
}

func TestDelegationUnavailableBridgeAndCrossConversationGrant(t *testing.T) {
	var absent *delegationFixture
	if _, err := NewDelegationClient(absent); !errors.Is(err, access.ErrUnavailable) {
		t.Fatal("missing bridge was enabled")
	}
	c, f := delegationTestClient(t)
	f.grant.ConversationID = "another-owner-conversation"
	if _, err := c.Issue(context.Background(), "session", "conversation-1", "turn-1"); !errors.Is(err, access.ErrUnavailable) {
		t.Fatal("mismatched issued scope was accepted")
	}
	if f.invokes != 0 {
		t.Fatal("issuance dispatched a model")
	}
}

func TestDelegationExactPayloadAndNoEconomicFields(t *testing.T) {
	c, _ := delegationTestClient(t)
	grant, err := c.Issue(context.Background(), "session", "conversation-1", "turn-1")
	if err != nil {
		t.Fatal(err)
	}
	request := ModelRequest{Input: "中文 <exact bytes>", Tools: []Definition{}, Results: []Result{}}
	operation, err := c.Prepare(grant, 0, request)
	if err != nil {
		t.Fatal(err)
	}
	encoded, _ := json.Marshal(request)
	sum := sha256.Sum256(encoded)
	if operation.RequestJSON != string(encoded) || operation.PayloadSHA256 != hex.EncodeToString(sum[:]) || operation.OperationKey != "turn-1:0" {
		t.Fatal("prepared binding changed the original payload bytes")
	}
	wire, _ := json.Marshal(operation)
	for _, field := range []string{`"payer"`, `"budget"`, `"model"`, `"price"`, `"token"`} {
		if bytes.Contains(wire, []byte(field)) {
			t.Fatal("consumer supplied Core economic authority")
		}
	}
	if _, err = c.Invoke(context.Background(), "session", operation); err != nil {
		t.Fatal(err)
	}
	operation.RequestJSON += " "
	if _, err = c.Invoke(context.Background(), "session", operation); !errors.Is(err, access.ErrConflict) {
		t.Fatal("mutated payload reached the bridge")
	}
}

func TestDelegationUnknownUsesOriginalStatusWithoutRetry(t *testing.T) {
	c, f := delegationTestClient(t)
	f.invokeError = io.EOF
	f.outcome = func(binding DelegationBinding) DelegationOutcome {
		return DelegationOutcome{DelegationBinding: binding, Status: "unknown"}
	}
	operation, err := c.Prepare(f.grant, 0, ModelRequest{Input: "question"})
	if err != nil {
		t.Fatal(err)
	}
	if _, err = c.Invoke(context.Background(), "session", operation); !errors.Is(err, ErrDelegationUnknown) {
		t.Fatal("lost ACK was treated as success")
	}
	c.now = func() time.Time { return time.Unix(2000, 0) }
	status, err := c.Status(context.Background(), "session", operation.DelegationBinding)
	if err != nil || status.Status != "unknown" || status.Reply != nil || f.invokes != 1 || f.issues != 0 || f.statuses != 1 {
		t.Fatal("recovery replaced the original operation or dispatched again")
	}
	if _, err = c.Prepare(f.grant, 1, ModelRequest{Input: "replacement"}); !errors.Is(err, access.ErrInvalid) {
		t.Fatal("expired grant prepared a new operation")
	}
}

func TestDelegationRejectsContradictoryOutcomes(t *testing.T) {
	c, f := delegationTestClient(t)
	operation, err := c.Prepare(f.grant, 0, ModelRequest{Input: "question"})
	if err != nil {
		t.Fatal(err)
	}
	valid := f.outcome
	mutations := []func(*DelegationOutcome){
		func(v *DelegationOutcome) { v.ConversationID = "other" },
		func(v *DelegationOutcome) { v.ID = "lmm_ad_" + strings.Repeat("b", 64) },
		func(v *DelegationOutcome) { v.PayloadSHA256 = strings.Repeat("b", 64) },
		func(v *DelegationOutcome) { v.Round = 1 },
		func(v *DelegationOutcome) { v.OperationKey = "turn-1:00" },
		func(v *DelegationOutcome) { v.Status = "success" },
		func(v *DelegationOutcome) { v.Status = "unknown" },
		func(v *DelegationOutcome) { v.CheckpointSequence = -1 },
		func(v *DelegationOutcome) { v.ChargedCredits = nil },
		func(v *DelegationOutcome) { amount := "9007199254740992"; v.ChargedCredits = &amount },
		func(v *DelegationOutcome) { amount := "01"; v.ChargedCredits = &amount },
		func(v *DelegationOutcome) { v.Reply.Calls = nil },
		func(v *DelegationOutcome) { v.Reply.Text = strings.Repeat("x", 8193) },
	}
	for index, mutate := range mutations {
		f.outcome = func(binding DelegationBinding) DelegationOutcome {
			response := valid(binding)
			mutate(&response)
			return response
		}
		if _, err := c.Invoke(context.Background(), "session", operation); !errors.Is(err, access.ErrUnavailable) {
			t.Fatalf("contradictory response %d was accepted", index)
		}
	}
}

func TestDelegationRejectsInvalidGrantAndRound(t *testing.T) {
	c, f := delegationTestClient(t)
	for _, round := range []int{-1, 4, 9} {
		if _, err := c.Prepare(f.grant, round, ModelRequest{Input: "question"}); !errors.Is(err, access.ErrInvalid) {
			t.Fatal("out-of-range round was accepted")
		}
	}
	f.grant.ExpiresAt = 1901
	if _, err := c.Issue(context.Background(), "session", "conversation-1", "turn-1"); !errors.Is(err, access.ErrUnavailable) {
		t.Fatal("excessive delegation lifetime was accepted")
	}
}

func TestDelegationPreservesDefiniteCoreRejection(t *testing.T) {
	c, f := delegationTestClient(t)
	operation, err := c.Prepare(f.grant, 0, ModelRequest{Input: "question"})
	if err != nil {
		t.Fatal(err)
	}
	for _, rejection := range []error{access.ErrInvalid, access.ErrUnauthorized, access.ErrForbidden, access.ErrConflict, access.ErrLimit, ErrDelegationBudget, ErrDelegationUnavailable} {
		f.issueError = rejection
		if _, err := c.Issue(context.Background(), "session", "conversation-1", "turn-1"); err != rejection {
			t.Fatal("definite issuance rejection became unknown")
		}
		f.invokeError = rejection
		if _, err := c.Invoke(context.Background(), "session", operation); err != rejection {
			t.Fatal("definite invocation rejection became unknown")
		}
	}
	f.invokeError = errors.New("provider-internal-credential")
	if _, err := c.Invoke(context.Background(), "session", operation); err != ErrDelegationUnknown {
		t.Fatal("unclassified error was exposed")
	}
}
