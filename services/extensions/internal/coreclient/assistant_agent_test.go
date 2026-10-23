package coreclient

import (
	"context"
	"crypto/sha256"
	pb "github.com/TokenNotIncluded/lmm-api/services/extensions/internal/corepb"
	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/metadata"
	"google.golang.org/grpc/status"
	"google.golang.org/protobuf/proto"
	"net"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

type agentPeer struct {
	pb.UnimplementedCoreControlServer
	pb.UnimplementedCoreAssistantModelServer
	feature bool
	mode    string
	calls   int
}

const agentHost = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
const agentSession = "ssssssssssssssssssssssssssssssss"

func (p *agentPeer) Capabilities(ctx context.Context, _ *pb.CapabilitiesRequest) (*pb.CapabilitiesResponse, error) {
	md, _ := metadata.FromIncomingContext(ctx)
	if len(md.Get("x-lmm-user-credential")) != 0 {
		return nil, status.Error(codes.Unauthenticated, "unexpected user")
	}
	features := []string{AssistantPersonalTextReserveFeature, AssistantPersonalTextExecuteFeature}
	if p.feature {
		features = []string{AssistantAgentTurnFeature}
	}
	return &pb.CapabilitiesResponse{ProtocolMajor: 1, Features: features}, nil
}
func (p *agentPeer) check(ctx context.Context) error {
	p.calls++
	md, _ := metadata.FromIncomingContext(ctx)
	if v := md.Get("authorization"); len(v) != 1 || v[0] != "Bearer "+agentHost {
		return status.Error(codes.Unauthenticated, "host")
	}
	if v := md.Get("x-lmm-user-credential"); len(v) != 1 || v[0] != agentSession {
		return status.Error(codes.Unauthenticated, "user")
	}
	return nil
}
func (p *agentPeer) BeginConversation(ctx context.Context, r *pb.AssistantConversationKey) (*pb.AssistantConversationReceipt, error) {
	if err := p.check(ctx); err != nil {
		return nil, err
	}
	return &pb.AssistantConversationReceipt{CreationKey: r.GetCreationKey(), CoreConversationId: "lmm_cv_" + strings.Repeat("c", 64)}, nil
}
func agentTurnFixture(cv, turn, text string) *pb.AssistantAgentTurn {
	h := sha256.Sum256([]byte(text))
	return &pb.AssistantAgentTurn{CoreConversationId: cv, ClientTurnId: turn, OwnerUserId: 1, OwnerAccountId: 2, OriginalTextSha256: h[:], Phase: pb.AssistantAgentPhase_ASSISTANT_AGENT_PHASE_MODEL_RUNNING, Revision: 1, MaxRounds: 4, ModelBudgetCredits: 10}
}
func (p *agentPeer) StartAgentTurn(ctx context.Context, r *pb.AssistantAgentStart) (*pb.AssistantAgentTurn, error) {
	if err := p.check(ctx); err != nil {
		return nil, err
	}
	out := agentTurnFixture(r.GetCoreConversationId(), r.GetClientTurnId(), r.GetText())
	switch p.mode {
	case "lost":
		return nil, status.Error(codes.Unavailable, "private detail")
	case "enum":
		out.Phase = 99
	case "unknown":
		out.ProtoReflect().SetUnknown([]byte{0x98, 0x06, 1})
	case "binding":
		out.ClientTurnId = "other"
	case "digest":
		out.OriginalTextSha256 = make([]byte, 32)
	case "credits":
		out.ModelHeldCredits = 11
	}
	return out, nil
}
func agentTestClient(t *testing.T, p *agentPeer) *AssistantClient {
	t.Helper()
	dir, err := os.MkdirTemp("", "agent-uds-")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { os.RemoveAll(dir) })
	socket := filepath.Join(dir, "rpc")
	l, err := net.Listen("unix", socket)
	if err != nil {
		t.Fatal(err)
	}
	server := grpc.NewServer()
	pb.RegisterCoreControlServer(server, p)
	pb.RegisterCoreAssistantModelServer(server, p)
	go server.Serve(l)
	t.Cleanup(server.Stop)
	control, err := New(socket, []byte(testToken))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { control.Close() })
	client, err := NewAssistantClient(socket, []byte(agentHost), control)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { client.Close() })
	return client
}
func TestAgentIndependentCapabilityAndSession(t *testing.T) {
	ctx := metadata.NewOutgoingContext(context.Background(), metadata.Pairs("authorization", "evil", "x-lmm-user-credential", "evil"))
	peer := &agentPeer{}
	client := agentTestClient(t, peer)
	r := &pb.AssistantAgentStart{CoreConversationId: "lmm_cv_" + strings.Repeat("c", 64), ClientTurnId: "turn", Text: "hello"}
	if _, err := client.AssistantStartAgentTurn(ctx, agentSession, r); status.Code(err) != codes.FailedPrecondition || peer.calls != 0 {
		t.Fatal(err, peer.calls)
	}
	peer = &agentPeer{feature: true}
	client = agentTestClient(t, peer)
	if _, err := client.AgentBeginConversation(ctx, agentSession, &pb.AssistantConversationKey{CreationKey: "registration"}); err != nil {
		t.Fatal(err)
	}
	if _, err := client.AssistantStartAgentTurn(ctx, agentSession, r); err != nil || peer.calls != 2 {
		t.Fatal(err, peer.calls)
	}
	if client.Ready(ctx) == nil {
		t.Fatal("agent implied paid-text capability")
	}
}
func TestAgentBadAcknowledgementNeverRetries(t *testing.T) {
	for _, mode := range []string{"lost", "enum", "unknown", "binding", "digest", "credits"} {
		t.Run(mode, func(t *testing.T) {
			p := &agentPeer{feature: true, mode: mode}
			c := agentTestClient(t, p)
			_, err := c.AssistantStartAgentTurn(context.Background(), agentSession, &pb.AssistantAgentStart{CoreConversationId: "lmm_cv_" + strings.Repeat("c", 64), ClientTurnId: "turn", Text: "hello"})
			if err == nil || p.calls != 1 {
				t.Fatal(err, p.calls)
			}
		})
	}
}
func TestAgentClosedNestedMessagesAndBounds(t *testing.T) {
	r := agentTurnFixture("lmm_cv_"+strings.Repeat("c", 64), "turn", "hello")
	if !agentTurn(r, r.CoreConversationId, r.ClientTurnId) {
		t.Fatal("fixture")
	}
	for _, mutate := range []func(*pb.AssistantAgentTurn){func(x *pb.AssistantAgentTurn) { x.MaxRounds = 33 }, func(x *pb.AssistantAgentTurn) { v := ""; x.FinalText = &v }, func(x *pb.AssistantAgentTurn) { x.Phase = 99 }} {
		x := proto.Clone(r).(*pb.AssistantAgentTurn)
		mutate(x)
		if agentTurn(x, x.CoreConversationId, x.ClientTurnId) {
			t.Fatal("accepted malformed response")
		}
	}
}

func agentPeerCall() *pb.AssistantAgentToolCall {
	args := []byte(`{"ok":true}`)
	h := sha256.Sum256(args)
	return &pb.AssistantAgentToolCall{ToolOperationId: "operation", ProviderCallId: "provider", ModelOperationId: "model", Scope: &pb.AssistantAgentScope{ToolName: "get_service_facts", ToolVersion: "v1", PriceVersion: "v1", SchemaSha256: make([]byte, 32), PriceSha256: make([]byte, 32), Domain: pb.AssistantAgentDomain_ASSISTANT_AGENT_DOMAIN_EXTENSION}, ArgumentsJson: args, ArgumentsSha256: h[:]}
}
func (p *agentPeer) ClaimAgentTool(ctx context.Context, r *pb.AssistantAgentClaim) (*pb.AssistantAgentClaimReceipt, error) {
	if err := p.check(ctx); err != nil {
		return nil, err
	}
	call := agentPeerCall()
	call.ToolOperationId = r.ToolOperationId
	out := &pb.AssistantAgentClaimReceipt{Call: call, ClaimKey: r.ClaimKey, Disposition: pb.AssistantAgentClaimDisposition_ASSISTANT_AGENT_CLAIM_INSPECT_ONLY}
	switch p.mode {
	case "binding":
		out.ClaimKey = "other"
	case "enum":
		out.Disposition = 99
	case "unknown":
		out.Call.Scope.ProtoReflect().SetUnknown([]byte{0x98, 0x06, 1})
	case "core-domain":
		out.Call.Scope.Domain = pb.AssistantAgentDomain_ASSISTANT_AGENT_DOMAIN_REGISTERED_CORE
	case "digest":
		out.Call.ArgumentsSha256 = make([]byte, 32)
	case "lost":
		return nil, status.Error(codes.Unavailable, "private")
	}
	return out, nil
}
func (p *agentPeer) CompleteAgentTool(ctx context.Context, r *pb.AssistantAgentComplete) (*pb.AssistantAgentResult, error) {
	if err := p.check(ctx); err != nil {
		return nil, err
	}
	out := &pb.AssistantAgentResult{ToolOperationId: r.ToolOperationId, ClaimKey: r.ClaimKey, State: r.State, ResultSha256: r.ResultSha256, DomainReceiptId: r.DomainReceiptId}
	switch p.mode {
	case "binding":
		out.ToolOperationId = "other"
	case "enum":
		out.State = 99
	case "unknown":
		out.ProtoReflect().SetUnknown([]byte{0x98, 0x06, 1})
	case "digest":
		out.ResultSha256 = make([]byte, 32)
	case "receipt":
		out.DomainReceiptId = "other"
	case "lost":
		return nil, status.Error(codes.Unavailable, "private")
	}
	return out, nil
}
func TestAgentClaimAndCompletionBindExactOriginalOperation(t *testing.T) {
	for _, mode := range []string{"", "binding", "enum", "unknown", "digest", "lost", "core-domain"} {
		t.Run(mode, func(t *testing.T) {
			p := &agentPeer{feature: true, mode: mode}
			c := agentTestClient(t, p)
			out, err := c.AssistantClaimAgentTool(context.Background(), agentSession, &pb.AssistantAgentClaim{ToolOperationId: "operation", ClaimKey: "claim"})
			if mode == "" {
				if err != nil || out.GetDisposition() != pb.AssistantAgentClaimDisposition_ASSISTANT_AGENT_CLAIM_INSPECT_ONLY {
					t.Fatal(out, err)
				}
			} else if err == nil {
				t.Fatal("malformed claim accepted")
			}
			if p.calls != 1 {
				t.Fatal("claim retried", p.calls)
			}
		})
	}
	for _, mode := range []string{"", "binding", "enum", "unknown", "digest", "receipt", "lost"} {
		t.Run("complete-"+mode, func(t *testing.T) {
			p := &agentPeer{feature: true, mode: mode}
			c := agentTestClient(t, p)
			data := []byte(`{"ok":true}`)
			h := sha256.Sum256(data)
			_, err := c.AssistantCompleteAgentTool(context.Background(), agentSession, &pb.AssistantAgentComplete{ToolOperationId: "operation", ClaimKey: "claim", State: pb.AssistantAgentResultState_ASSISTANT_AGENT_RESULT_COMPLETED, ResultJson: data, ResultSha256: h[:], DomainReceiptId: "domain-receipt"})
			if (mode == "") != (err == nil) || p.calls != 1 {
				t.Fatal(err, p.calls)
			}
		})
	}
}
func TestAgentRejectsBadRequestsBeforeRPC(t *testing.T) {
	p := &agentPeer{feature: true}
	c := agentTestClient(t, p)
	r := &pb.AssistantAgentComplete{ToolOperationId: "op", ClaimKey: "claim", State: pb.AssistantAgentResultState_ASSISTANT_AGENT_RESULT_COMPLETED, ResultJson: []byte(`{}`), ResultSha256: make([]byte, 32), DomainReceiptId: "receipt"}
	if _, err := c.AssistantCompleteAgentTool(context.Background(), agentSession, r); status.Code(err) != codes.InvalidArgument || p.calls != 0 {
		t.Fatal(err, p.calls)
	}
	r.ProtoReflect().SetUnknown([]byte{0x98, 0x06, 1})
	if _, err := c.AssistantCompleteAgentTool(context.Background(), agentSession, r); status.Code(err) != codes.InvalidArgument || p.calls != 0 {
		t.Fatal(err, p.calls)
	}
}
