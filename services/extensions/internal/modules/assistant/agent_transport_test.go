package assistant

import (
	"encoding/hex"
	"errors"
	pb "github.com/TokenNotIncluded/api.lmm.best/extensions/internal/corepb"
	"strings"
	"testing"
)

func agentTransportCallFixture() *pb.AssistantAgentToolCall {
	args := []byte(`{"query":"hello"}`)
	d, _ := hex.DecodeString(agentDigest(args))
	h := make([]byte, 32)
	return &pb.AssistantAgentToolCall{ToolOperationId: "operation", ProviderCallId: "provider-call", ModelOperationId: "model-operation", Scope: &pb.AssistantAgentScope{ToolName: "search_web", ToolVersion: "v1", PriceVersion: "v1", SchemaSha256: h, PriceSha256: h, Domain: pb.AssistantAgentDomain_ASSISTANT_AGENT_DOMAIN_EXTENSION}, ArgumentsJson: args, ArgumentsSha256: d}
}
func TestAgentTransportMapsExactScopeAndAccount(t *testing.T) {
	key := AgentTurnKey{"lmm_cv_" + strings.Repeat("c", 64), "turn"}
	hash, _ := hex.DecodeString(agentDigest([]byte("hello")))
	r := &pb.AssistantAgentTurn{CoreConversationId: key.CoreConversationID, ClientTurnId: key.ClientTurnID, OwnerUserId: 7, OwnerAccountId: 19, OriginalTextSha256: hash, Phase: pb.AssistantAgentPhase_ASSISTANT_AGENT_PHASE_TOOLS_PENDING, Revision: 1, MaxRounds: 4, ModelBudgetCredits: 10, PendingCalls: []*pb.AssistantAgentToolCall{agentTransportCallFixture()}}
	out, err := agentSnapshotFromRPC(key, r)
	if err != nil || out.Account.ID != 7 || out.CoreOwnerAccountID != 19 || len(out.PendingTools) != 1 || out.PendingTools[0].CoreConversationID != key.CoreConversationID || out.PendingTools[0].Scope.SchemaSHA256 != strings.Repeat("0", 64) {
		t.Fatal(out, err)
	}
	r.PendingCalls[0].ArgumentsJson[0] = 'x'
	if string(out.PendingTools[0].Arguments) == string(r.PendingCalls[0].ArgumentsJson) {
		t.Fatal("untrusted protobuf buffer aliased")
	}
}
func TestAgentTransportRejectsAuthorityAndBindingSubstitution(t *testing.T) {
	key := AgentTurnKey{"lmm_cv_" + strings.Repeat("c", 64), "turn"}
	for _, mutate := range []func(*pb.AssistantAgentToolCall){func(c *pb.AssistantAgentToolCall) {
		c.Scope.Domain = pb.AssistantAgentDomain_ASSISTANT_AGENT_DOMAIN_REGISTERED_CORE
	}, func(c *pb.AssistantAgentToolCall) { c.ArgumentsSha256 = make([]byte, 32) }, func(c *pb.AssistantAgentToolCall) { c.Scope.ToolName = "call_market_tool" }, func(c *pb.AssistantAgentToolCall) { c.Round = 32 }} {
		r := agentTransportCallFixture()
		mutate(r)
		if _, err := agentCallFromRPC(key, r); !errors.Is(err, ErrDelegationUnknown) {
			t.Fatal("accepted substituted call", err)
		}
	}
}
func TestAgentTransportResultDoesNotInventSuccess(t *testing.T) {
	hash := make([]byte, 32)
	r := &pb.AssistantAgentResult{ToolOperationId: "op", ClaimKey: "claim", State: pb.AssistantAgentResultState_ASSISTANT_AGENT_RESULT_UNKNOWN, ResultSha256: hash}
	out, err := agentReceiptFromRPC(r)
	if err != nil || out.State != AgentToolUnknown {
		t.Fatal(out, err)
	}
	r.State = pb.AssistantAgentResultState_ASSISTANT_AGENT_RESULT_COMPLETED
	if _, err := agentReceiptFromRPC(r); err == nil {
		t.Fatal("completion without business receipt")
	}
	r.DomainReceiptId = "receipt"
	if _, err := agentReceiptFromRPC(r); err != nil {
		t.Fatal(err)
	}
	r.State = 99
	if _, err := agentReceiptFromRPC(r); err == nil {
		t.Fatal("unknown enum")
	}
}
