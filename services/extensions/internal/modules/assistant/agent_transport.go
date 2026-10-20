package assistant

import (
	"context"
	"encoding/hex"
	"unicode/utf8"

	"github.com/TokenNotIncluded/api.lmm.best/extensions/internal/coreclient"
	pb "github.com/TokenNotIncluded/api.lmm.best/extensions/internal/corepb"
	"github.com/TokenNotIncluded/api.lmm.best/extensions/internal/modules/assistant/access"
)

type CoreAgentTurnPort struct{ client *coreclient.AssistantClient }

var _ AgentTurnPort = (*CoreAgentTurnPort)(nil)
var _ ConversationRegistrationPort = (*CoreAgentTurnPort)(nil)

func NewCoreAgentTurnPort(client *coreclient.AssistantClient) (*CoreAgentTurnPort, error) {
	if client == nil {
		return nil, ErrDelegationUnavailable
	}
	return &CoreAgentTurnPort{client}, nil
}
func (p *CoreAgentTurnPort) Ready(ctx context.Context) error { return p.client.AgentReady(ctx) }
func (p *CoreAgentTurnPort) BeginConversation(ctx context.Context, credential, key string) (CoreConversationRegistration, error) {
	r, err := p.client.AgentBeginConversation(ctx, credential, &pb.AssistantConversationKey{CreationKey: key})
	if err != nil {
		return CoreConversationRegistration{}, personalTextRPCError(err)
	}
	return personalConversationReceipt(key, r)
}
func (p *CoreAgentTurnPort) LookupConversation(ctx context.Context, credential, key string) (CoreConversationRegistration, error) {
	r, err := p.client.AgentLookupConversation(ctx, credential, &pb.AssistantConversationKey{CreationKey: key})
	if err != nil {
		return CoreConversationRegistration{}, personalTextRPCError(err)
	}
	return personalConversationReceipt(key, r)
}
func agentScopeFromRPC(s *pb.AssistantAgentScope) AgentToolScope {
	return AgentToolScope{Name: s.GetToolName(), Version: s.GetToolVersion(), SchemaSHA256: hex.EncodeToString(s.GetSchemaSha256()), PriceVersion: s.GetPriceVersion(), PriceSHA256: hex.EncodeToString(s.GetPriceSha256()), Domain: "extension", ResourceKind: s.GetResourceKind(), ResourceID: s.GetResourceId(), ResourceVersion: s.GetResourceVersion(), ResourceSchemaSHA256: hex.EncodeToString(s.GetResourceSchemaSha256()), ResourcePriceSHA256: hex.EncodeToString(s.GetResourcePriceSha256()), GrantID: s.GetGrantId()}
}

// key is a local journal index, not authority returned by this Claim RPC.
// Core binds the operation to the original Session; the consumer must compare
// the complete returned call with its immutable pending-call journal.
func agentCallFromRPC(key AgentTurnKey, r *pb.AssistantAgentToolCall) (AgentToolCall, error) {
	if r == nil || r.GetScope() == nil || r.GetScope().GetDomain() != pb.AssistantAgentDomain_ASSISTANT_AGENT_DOMAIN_EXTENSION {
		return AgentToolCall{}, ErrDelegationUnknown
	}
	c := AgentToolCall{OperationID: r.GetToolOperationId(), CoreConversationID: key.CoreConversationID, TurnID: key.ClientTurnID, Round: r.GetRound(), Ordinal: r.GetOrdinal(), ModelOperationID: r.GetModelOperationId(), ProviderCallID: r.GetProviderCallId(), Scope: agentScopeFromRPC(r.GetScope()), Arguments: append([]byte(nil), r.GetArgumentsJson()...), ArgumentsSHA256: hex.EncodeToString(r.GetArgumentsSha256())}
	if !validAgentCall(c) {
		return AgentToolCall{}, ErrDelegationUnknown
	}
	return c, nil
}
func agentSnapshotFromRPC(key AgentTurnKey, r *pb.AssistantAgentTurn) (AgentTurnSnapshot, error) {
	if r == nil {
		return AgentTurnSnapshot{}, ErrDelegationUnknown
	}
	phases := map[pb.AssistantAgentPhase]AgentTurnPhase{1: AgentTurnNew, 2: AgentTurnRunning, 3: AgentTurnAwaitingTools, 4: AgentTurnCompleted, 5: AgentTurnUnknown, 6: AgentTurnFailed, 7: AgentTurnExpired}
	s := AgentTurnSnapshot{Key: AgentTurnKey{r.GetCoreConversationId(), r.GetClientTurnId()}, OwnerUserID: r.GetOwnerUserId(), CoreOwnerAccountID: r.GetOwnerAccountId(), Account: access.Account{Kind: "personal", ID: r.GetOwnerUserId()}, OriginalTextSHA256: hex.EncodeToString(r.GetOriginalTextSha256()), Revision: r.GetRevision(), Round: r.GetRound(), MaxRounds: r.GetMaxRounds(), Phase: phases[r.GetPhase()], BudgetCredits: r.GetModelBudgetCredits(), HeldCredits: r.GetModelHeldCredits(), SettledCredits: r.GetModelSettledCredits(), FinalText: r.GetFinalText()}
	for _, call := range r.GetPendingCalls() {
		c, err := agentCallFromRPC(key, call)
		if err != nil {
			return AgentTurnSnapshot{}, err
		}
		s.PendingTools = append(s.PendingTools, c)
	}
	if !validAgentSnapshot(s, key, s.OwnerUserID, s.OriginalTextSHA256) {
		return AgentTurnSnapshot{}, ErrDelegationUnknown
	}
	return s, nil
}
func (p *CoreAgentTurnPort) StartAgentTurn(ctx context.Context, credential string, in AgentTurnStart) (AgentTurnSnapshot, error) {
	if !validAgentKey(in.Key) || len(in.Text) == 0 || len(in.Text) > 8192 || !utf8.ValidString(in.Text) {
		return AgentTurnSnapshot{}, access.ErrInvalid
	}
	r, err := p.client.AssistantStartAgentTurn(ctx, credential, &pb.AssistantAgentStart{CoreConversationId: in.Key.CoreConversationID, ClientTurnId: in.Key.ClientTurnID, Text: in.Text})
	if err != nil {
		return AgentTurnSnapshot{}, personalTextRPCError(err)
	}
	s, err := agentSnapshotFromRPC(in.Key, r)
	if err == nil && s.OriginalTextSHA256 != agentDigest([]byte(in.Text)) {
		return AgentTurnSnapshot{}, ErrDelegationUnknown
	}
	return s, err
}
func (p *CoreAgentTurnPort) GetAgentTurn(ctx context.Context, credential string, key AgentTurnKey) (AgentTurnSnapshot, error) {
	if !validAgentKey(key) {
		return AgentTurnSnapshot{}, access.ErrInvalid
	}
	r, err := p.client.AssistantGetAgentTurn(ctx, credential, &pb.AssistantAgentLookup{CoreConversationId: key.CoreConversationID, ClientTurnId: key.ClientTurnID})
	if err != nil {
		return AgentTurnSnapshot{}, personalTextRPCError(err)
	}
	return agentSnapshotFromRPC(key, r)
}
func agentReceiptFromRPC(r *pb.AssistantAgentResult) (AgentToolReceipt, error) {
	if r == nil {
		return AgentToolReceipt{}, ErrDelegationUnknown
	}
	state := AgentToolUnknown
	switch r.GetState() {
	case pb.AssistantAgentResultState_ASSISTANT_AGENT_RESULT_COMPLETED:
		state = AgentToolSucceeded
	case pb.AssistantAgentResultState_ASSISTANT_AGENT_RESULT_UNKNOWN:
	default:
		return AgentToolReceipt{}, ErrDelegationUnknown
	}
	out := AgentToolReceipt{OperationID: r.GetToolOperationId(), ClaimKey: r.GetClaimKey(), State: state, ResultSHA256: hex.EncodeToString(r.GetResultSha256()), DomainReceiptID: r.GetDomainReceiptId()}
	if !access.ID(out.OperationID) || !access.ID(out.ClaimKey) || !lowerDigest(out.ResultSHA256) || (out.DomainReceiptID != "" && !delegationLabel(out.DomainReceiptID, 256)) || (state == AgentToolSucceeded && out.DomainReceiptID == "") {
		return AgentToolReceipt{}, ErrDelegationUnknown
	}
	return out, nil
}
func (p *CoreAgentTurnPort) ClaimAgentTool(ctx context.Context, credential string, in AgentToolClaimInput) (AgentToolClaim, error) {
	if !validAgentKey(in.Key) || !access.ID(in.OperationID) || !access.ID(in.ClaimKey) {
		return AgentToolClaim{}, access.ErrInvalid
	}
	r, err := p.client.AssistantClaimAgentTool(ctx, credential, &pb.AssistantAgentClaim{ToolOperationId: in.OperationID, ClaimKey: in.ClaimKey})
	if err != nil {
		return AgentToolClaim{}, personalTextRPCError(err)
	}
	call, err := agentCallFromRPC(in.Key, r.GetCall())
	if err != nil {
		return AgentToolClaim{}, err
	}
	out := AgentToolClaim{Call: call, ClaimKey: r.GetClaimKey(), MayExecute: r.GetDisposition() == pb.AssistantAgentClaimDisposition_ASSISTANT_AGENT_CLAIM_EXECUTE_ONCE}
	if r.GetAcceptedResult() != nil {
		a, err := agentReceiptFromRPC(r.GetAcceptedResult())
		if err != nil {
			return AgentToolClaim{}, err
		}
		out.AcceptedResult = &a
	}
	return out, nil
}
func (p *CoreAgentTurnPort) CompleteAgentTool(ctx context.Context, credential string, in AgentToolCompletion) (AgentToolReceipt, error) {
	if !validAgentKey(in.Claim.Key) || !access.ID(in.Claim.OperationID) || !access.ID(in.Claim.ClaimKey) || in.Result.OperationID != in.Claim.OperationID || !agentJSONObject(in.Result.ResultJSON) || agentDigest(in.Result.ResultJSON) != in.Result.ResultSHA256 || in.Result.FinalText != "" {
		return AgentToolReceipt{}, access.ErrInvalid
	}
	state := pb.AssistantAgentResultState_ASSISTANT_AGENT_RESULT_UNKNOWN
	switch in.Result.State {
	case AgentToolSucceeded, AgentToolFailed:
		state = pb.AssistantAgentResultState_ASSISTANT_AGENT_RESULT_COMPLETED
	case AgentToolUnknown:
	default:
		return AgentToolReceipt{}, access.ErrInvalid
	}
	digest, _ := hex.DecodeString(in.Result.ResultSHA256)
	r, err := p.client.AssistantCompleteAgentTool(ctx, credential, &pb.AssistantAgentComplete{ToolOperationId: in.Claim.OperationID, ClaimKey: in.Claim.ClaimKey, State: state, ResultJson: append([]byte(nil), in.Result.ResultJSON...), ResultSha256: digest, DomainReceiptId: in.Result.DomainReceiptID})
	if err != nil {
		return AgentToolReceipt{}, personalTextRPCError(err)
	}
	out, err := agentReceiptFromRPC(r)
	if err == nil && !validAgentToolReceipt(AgentToolCall{OperationID: in.Claim.OperationID}, in.Claim.ClaimKey, in.Result, out) {
		return AgentToolReceipt{}, ErrDelegationUnknown
	}
	return out, err
}
