package coreclient

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/json"
	pb "github.com/TokenNotIncluded/api.lmm.best/extensions/internal/corepb"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
	"regexp"
	"strings"
	"unicode/utf8"
)

const AssistantAgentTurnFeature = "assistant.agent_turn.execute.v1"

var agentIdentifier = regexp.MustCompile(`^[a-zA-Z0-9][a-zA-Z0-9_.:-]{0,127}$`)

func (c *AssistantClient) AgentReady(ctx context.Context) error {
	return c.assistantFeatures(ctx, AssistantAgentTurnFeature)
}
func (c *AssistantClient) agentReady(ctx context.Context, user string) error {
	if !validUserCredential(user) {
		return status.Error(codes.Unauthenticated, "invalid user credential")
	}
	return c.AgentReady(ctx)
}
func agentInvalid() error { return status.Error(codes.InvalidArgument, "invalid agent request") }
func agentCorrupt() error { return status.Error(codes.DataLoss, "invalid agent response") }
func agentKey(cv, turn string) bool {
	return assistantDigest(cv, "lmm_cv_") && agentIdentifier.MatchString(turn) && len(turn) <= 64
}
func agentJSON(data []byte) bool {
	return len(data) > 0 && len(data) <= 65536 && utf8.Valid(data) && strings.HasPrefix(strings.TrimSpace(string(data)), "{") && json.Valid(data)
}
func agentResult(r *pb.AssistantAgentResult) bool {
	return assistantMessage(r) && agentIdentifier.MatchString(r.GetToolOperationId()) && agentIdentifier.MatchString(r.GetClaimKey()) && (r.GetState() == pb.AssistantAgentResultState_ASSISTANT_AGENT_RESULT_COMPLETED || r.GetState() == pb.AssistantAgentResultState_ASSISTANT_AGENT_RESULT_UNKNOWN) && len(r.GetResultSha256()) == 32 && (r.GetDomainReceiptId() == "" || assistantLabel(r.GetDomainReceiptId(), 256)) && (r.GetState() != pb.AssistantAgentResultState_ASSISTANT_AGENT_RESULT_COMPLETED || r.GetDomainReceiptId() != "")
}
func agentCall(r *pb.AssistantAgentToolCall) bool {
	if !assistantMessage(r) || !agentIdentifier.MatchString(r.GetToolOperationId()) || !agentIdentifier.MatchString(r.GetProviderCallId()) || !agentIdentifier.MatchString(r.GetModelOperationId()) || r.GetRound() >= 32 || r.GetOrdinal() >= 8 || !agentJSON(r.GetArgumentsJson()) {
		return false
	}
	h := sha256.Sum256(r.GetArgumentsJson())
	if !bytes.Equal(h[:], r.GetArgumentsSha256()) {
		return false
	}
	s := r.GetScope()
	if s == nil || s.GetDomain() != pb.AssistantAgentDomain_ASSISTANT_AGENT_DOMAIN_EXTENSION || !assistantLabel(s.GetToolName(), 80) || !assistantLabel(s.GetToolVersion(), 128) || !assistantLabel(s.GetPriceVersion(), 128) || len(s.GetSchemaSha256()) != 32 || len(s.GetPriceSha256()) != 32 {
		return false
	}
	for _, v := range []string{s.GetResourceKind(), s.GetResourceId(), s.GetResourceVersion(), s.GetGrantId()} {
		if v != "" && !assistantLabel(v, 128) {
			return false
		}
	}
	for _, v := range [][]byte{s.GetResourceSchemaSha256(), s.GetResourcePriceSha256()} {
		if len(v) != 0 && len(v) != 32 {
			return false
		}
	}
	return s.GetToolName() != "call_market_tool" || (s.GetResourceKind() == "toolmarket" && s.GetResourceId() != "" && s.GetResourceVersion() != "" && s.GetGrantId() != "" && len(s.GetResourceSchemaSha256()) == 32 && len(s.GetResourcePriceSha256()) == 32)
}
func agentTurn(r *pb.AssistantAgentTurn, cv, turn string) bool {
	const safe int64 = 9007199254740991
	if !assistantMessage(r) || r.GetCoreConversationId() != cv || r.GetClientTurnId() != turn || !agentKey(cv, turn) || r.GetOwnerUserId() <= 0 || r.GetOwnerAccountId() <= 0 || len(r.GetOriginalTextSha256()) != 32 || r.GetRevision() == 0 || r.GetMaxRounds() == 0 || r.GetMaxRounds() > 32 || r.GetRound() >= r.GetMaxRounds() || r.GetModelBudgetCredits() <= 0 || r.GetModelBudgetCredits() > safe || r.GetModelHeldCredits() < 0 || r.GetModelSettledCredits() < 0 || r.GetModelSettledCredits() > r.GetModelBudgetCredits() || r.GetModelHeldCredits() > r.GetModelBudgetCredits()-r.GetModelSettledCredits() || len(r.GetPendingCalls()) > 8 || r.GetPhase() == 0 {
		return false
	}
	if r.GetPhase() == pb.AssistantAgentPhase_ASSISTANT_AGENT_PHASE_COMPLETED {
		if r.FinalText == nil || len(r.GetFinalText()) == 0 || len(r.GetFinalText()) > 32768 || !utf8.ValidString(r.GetFinalText()) {
			return false
		}
	} else if r.FinalText != nil {
		return false
	}
	if r.GetPhase() != pb.AssistantAgentPhase_ASSISTANT_AGENT_PHASE_TOOLS_PENDING && len(r.GetPendingCalls()) != 0 {
		return false
	}
	if r.GetPhase() == pb.AssistantAgentPhase_ASSISTANT_AGENT_PHASE_TOOLS_PENDING && len(r.GetPendingCalls()) == 0 {
		return false
	}
	seen := map[string]bool{}
	ord := map[uint32]bool{}
	for _, call := range r.GetPendingCalls() {
		if !agentCall(call) || call.GetRound() != r.GetRound() || seen[call.GetToolOperationId()] || ord[call.GetOrdinal()] {
			return false
		}
		seen[call.GetToolOperationId()] = true
		ord[call.GetOrdinal()] = true
	}
	return true
}
func (c *AssistantClient) AgentBeginConversation(ctx context.Context, user string, r *pb.AssistantConversationKey) (*pb.AssistantConversationReceipt, error) {
	return c.agentConversation(ctx, user, r, false)
}
func (c *AssistantClient) AgentLookupConversation(ctx context.Context, user string, r *pb.AssistantConversationKey) (*pb.AssistantConversationReceipt, error) {
	return c.agentConversation(ctx, user, r, true)
}
func (c *AssistantClient) agentConversation(ctx context.Context, user string, r *pb.AssistantConversationKey, lookup bool) (*pb.AssistantConversationReceipt, error) {
	if !assistantMessage(r) || !assistantLabel(r.GetCreationKey(), 128) {
		return nil, agentInvalid()
	}
	if err := c.agentReady(ctx, user); err != nil {
		return nil, err
	}
	client := pb.NewCoreAssistantModelClient(c.rpc.connection)
	var out *pb.AssistantConversationReceipt
	var err error
	if lookup {
		out, err = client.LookupConversation(c.rpc.context(ctx, user), r)
	} else {
		out, err = client.BeginConversation(c.rpc.context(ctx, user), r)
	}
	if err != nil {
		return nil, err
	}
	if !assistantMessage(out) || out.GetCreationKey() != r.GetCreationKey() || !assistantDigest(out.GetCoreConversationId(), "lmm_cv_") {
		return nil, agentCorrupt()
	}
	return out, nil
}
func (c *AssistantClient) AssistantStartAgentTurn(ctx context.Context, user string, r *pb.AssistantAgentStart) (*pb.AssistantAgentTurn, error) {
	if !assistantMessage(r) || !agentKey(r.GetCoreConversationId(), r.GetClientTurnId()) || len(r.GetText()) == 0 || len(r.GetText()) > 8192 || !utf8.ValidString(r.GetText()) {
		return nil, agentInvalid()
	}
	if err := c.agentReady(ctx, user); err != nil {
		return nil, err
	}
	out, err := pb.NewCoreAssistantModelClient(c.rpc.connection).StartAgentTurn(c.rpc.context(ctx, user), r)
	if err != nil {
		return nil, err
	}
	h := sha256.Sum256([]byte(r.GetText()))
	if !agentTurn(out, r.GetCoreConversationId(), r.GetClientTurnId()) || !bytes.Equal(h[:], out.GetOriginalTextSha256()) {
		return nil, agentCorrupt()
	}
	return out, nil
}
func (c *AssistantClient) AssistantGetAgentTurn(ctx context.Context, user string, r *pb.AssistantAgentLookup) (*pb.AssistantAgentTurn, error) {
	if !assistantMessage(r) || !agentKey(r.GetCoreConversationId(), r.GetClientTurnId()) {
		return nil, agentInvalid()
	}
	if err := c.agentReady(ctx, user); err != nil {
		return nil, err
	}
	out, err := pb.NewCoreAssistantModelClient(c.rpc.connection).GetAgentTurn(c.rpc.context(ctx, user), r)
	if err != nil {
		return nil, err
	}
	if !agentTurn(out, r.GetCoreConversationId(), r.GetClientTurnId()) {
		return nil, agentCorrupt()
	}
	return out, nil
}
func (c *AssistantClient) AssistantClaimAgentTool(ctx context.Context, user string, r *pb.AssistantAgentClaim) (*pb.AssistantAgentClaimReceipt, error) {
	if !assistantMessage(r) || !agentIdentifier.MatchString(r.GetToolOperationId()) || !agentIdentifier.MatchString(r.GetClaimKey()) {
		return nil, agentInvalid()
	}
	if err := c.agentReady(ctx, user); err != nil {
		return nil, err
	}
	out, err := pb.NewCoreAssistantModelClient(c.rpc.connection).ClaimAgentTool(c.rpc.context(ctx, user), r)
	if err != nil {
		return nil, err
	}
	if !assistantMessage(out) || !agentCall(out.GetCall()) || out.GetCall().GetToolOperationId() != r.GetToolOperationId() || out.GetClaimKey() != r.GetClaimKey() || (out.GetDisposition() != pb.AssistantAgentClaimDisposition_ASSISTANT_AGENT_CLAIM_EXECUTE_ONCE && out.GetDisposition() != pb.AssistantAgentClaimDisposition_ASSISTANT_AGENT_CLAIM_INSPECT_ONLY) {
		return nil, agentCorrupt()
	}
	if a := out.GetAcceptedResult(); a != nil && (!agentResult(a) || a.GetToolOperationId() != r.GetToolOperationId() || a.GetClaimKey() != r.GetClaimKey() || out.GetDisposition() != pb.AssistantAgentClaimDisposition_ASSISTANT_AGENT_CLAIM_INSPECT_ONLY) {
		return nil, agentCorrupt()
	}
	return out, nil
}
func (c *AssistantClient) AssistantCompleteAgentTool(ctx context.Context, user string, r *pb.AssistantAgentComplete) (*pb.AssistantAgentResult, error) {
	if !assistantMessage(r) || !agentJSON(r.GetResultJson()) {
		return nil, agentInvalid()
	}
	h := sha256.Sum256(r.GetResultJson())
	if !bytes.Equal(h[:], r.GetResultSha256()) || !agentResult(&pb.AssistantAgentResult{ToolOperationId: r.GetToolOperationId(), ClaimKey: r.GetClaimKey(), State: r.GetState(), ResultSha256: r.GetResultSha256(), DomainReceiptId: r.GetDomainReceiptId()}) {
		return nil, agentInvalid()
	}
	if err := c.agentReady(ctx, user); err != nil {
		return nil, err
	}
	out, err := pb.NewCoreAssistantModelClient(c.rpc.connection).CompleteAgentTool(c.rpc.context(ctx, user), r)
	if err != nil {
		return nil, err
	}
	if !agentResult(out) || out.GetToolOperationId() != r.GetToolOperationId() || out.GetClaimKey() != r.GetClaimKey() || out.GetState() != r.GetState() || !bytes.Equal(out.GetResultSha256(), r.GetResultSha256()) || out.GetDomainReceiptId() != r.GetDomainReceiptId() {
		return nil, agentCorrupt()
	}
	return out, nil
}
