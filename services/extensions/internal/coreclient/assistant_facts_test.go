package coreclient

import (
	"context"
	"fmt"
	"net"
	"os"
	"path/filepath"
	"strings"
	"testing"

	pb "github.com/TokenNotIncluded/lmm-api/services/extensions/internal/corepb"
	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/metadata"
	"google.golang.org/grpc/status"
)

type factsPeer struct {
	agentPeer
	enabled   bool
	authCalls int
}

func (p *factsPeer) Capabilities(ctx context.Context, _ *pb.CapabilitiesRequest) (*pb.CapabilitiesResponse, error) {
	md, _ := metadata.FromIncomingContext(ctx)
	if len(md.Get("x-lmm-user-credential")) != 0 {
		return nil, status.Error(codes.Unauthenticated, "unexpected user")
	}
	features := []string{AssistantAgentTurnFeature}
	if p.enabled {
		features = []string{AssistantBusinessFactsFeature}
	}
	return &pb.CapabilitiesResponse{ProtocolMajor: 1, Features: features}, nil
}
func (p *factsPeer) Authorize(ctx context.Context, _ *pb.AuthorizeRequest) (*pb.AuthorizeResponse, error) {
	p.authCalls++
	md, _ := metadata.FromIncomingContext(ctx)
	if v := md.Get("authorization"); len(v) != 1 || v[0] != "Bearer "+testToken {
		return nil, status.Error(codes.Unauthenticated, "control")
	}
	if v := md.Get("x-lmm-user-credential"); len(v) != 1 || v[0] != agentSession {
		return nil, status.Error(codes.Unauthenticated, "session")
	}
	out := &pb.AuthorizeResponse{UserId: 7, CredentialKind: pb.CredentialKind_CREDENTIAL_KIND_SESSION, Owner: &pb.Account{Kind: pb.AccountKind_ACCOUNT_KIND_PERSONAL, Id: 7}}
	switch p.mode {
	case "api-key":
		out.CredentialKind = pb.CredentialKind_CREDENTIAL_KIND_API_KEY
	case "team":
		out.Owner.Kind = pb.AccountKind_ACCOUNT_KIND_TEAM
	case "unknown-actor":
		out.Owner.ProtoReflect().SetUnknown([]byte{0x98, 0x06, 1})
	}
	return out, nil
}
func (p *factsPeer) GetAssistantBusinessFacts(ctx context.Context, r *pb.GetAssistantBusinessFactsRequest) (*pb.GetAssistantBusinessFactsResponse, error) {
	if err := p.check(ctx); err != nil {
		return nil, err
	}
	raw := `{"actor_user_id":7,"exact_model_ids":["vendor/model"],"developer_access_granted":true,"locked":false,"public_preview":false,"key_group_options":["default"],"catalogue":null}`
	if r.Kind == pb.AssistantBusinessFactKind_ASSISTANT_BUSINESS_FACT_KIND_SUPPORT_ELIGIBILITY {
		raw = `{"ok":true,"scope":"self","actor_user_id":7,"eligible":true,"successful_external_payments":1,"unrefunded_amount_minor":"900719925474099312345","currency":"USD","evidence":"core verified external payment receipts","wallet_balance_is_not_qualification":true,"checked_at_unix":123456,"projection_id":"support-observation-v1:` + strings.Repeat("a", 64) + `","projection_kind":"authenticated_read_observation_not_financial_receipt"}`
	}
	switch p.mode {
	case "actor":
		raw = strings.Replace(raw, `"actor_user_id":7`, `"actor_user_id":8`, 1)
	case "unknown-json":
		raw = raw[:len(raw)-1] + `,"payer":1}`
	case "duplicate":
		raw = raw[:len(raw)-1] + `,"actor_user_id":7}`
	case "oversized":
		raw = `{"body":"` + strings.Repeat("a", maxAssistantBusinessFacts) + `"}`
	case "non-object":
		raw = `[]`
	case "lost":
		return nil, status.Error(codes.Unavailable, "private detail")
	}
	out := &pb.GetAssistantBusinessFactsResponse{FactsJson: []byte(raw)}
	if p.mode == "unknown" {
		out.ProtoReflect().SetUnknown([]byte{0x98, 0x06, 1})
	}
	return out, nil
}
func factsTestClient(t *testing.T, p *factsPeer) *AssistantClient {
	t.Helper()
	dir, err := os.MkdirTemp("", "facts-uds-")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { os.RemoveAll(dir) })
	socket := filepath.Join(dir, "rpc")
	l, err := net.Listen("unix", socket)
	if err != nil {
		t.Fatal(err)
	}
	s := grpc.NewServer()
	pb.RegisterCoreControlServer(s, p)
	pb.RegisterCoreAssistantModelServer(s, p)
	go s.Serve(l)
	t.Cleanup(s.Stop)
	control, err := New(socket, []byte(testToken))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { control.Close() })
	c, err := NewAssistantClient(socket, []byte(agentHost), control)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { c.Close() })
	return c
}
func TestBusinessFactsIndependentCapabilityClosedKindAndSession(t *testing.T) {
	p := &factsPeer{}
	c := factsTestClient(t, p)
	for _, kind := range []string{"", " arbitrary", "get_admin_operations", "AVAILABLE_MODELS"} {
		if _, err := c.GetAssistantBusinessFacts(context.Background(), agentSession, kind); status.Code(err) != codes.InvalidArgument || p.calls != 0 || p.authCalls != 0 {
			t.Fatal(err, p.calls, p.authCalls)
		}
	}
	if _, err := c.GetAssistantBusinessFacts(context.Background(), agentSession, "available_models"); status.Code(err) != codes.FailedPrecondition || p.calls != 0 || p.authCalls != 0 {
		t.Fatal(err, p.calls, p.authCalls)
	}
	p = &factsPeer{enabled: true}
	c = factsTestClient(t, p)
	ctx := metadata.NewOutgoingContext(context.Background(), metadata.Pairs("authorization", "evil", "x-lmm-user-credential", "evil"))
	for _, kind := range []string{"available_models", "support_eligibility"} {
		raw, err := c.GetAssistantBusinessFacts(ctx, agentSession, kind)
		if err != nil || len(raw) == 0 {
			t.Fatal(fmt.Sprint(raw), err)
		}
	}
	if p.calls != 2 || p.authCalls != 2 {
		t.Fatal(p.calls, p.authCalls)
	}
}
func TestBusinessFactsRejectsSubstitutionAndDoesNotRetry(t *testing.T) {
	for _, mode := range []string{"api-key", "team", "unknown-actor", "actor", "unknown", "unknown-json", "duplicate", "oversized", "non-object", "lost"} {
		t.Run(mode, func(t *testing.T) {
			p := &factsPeer{enabled: true}
			p.mode = mode
			c := factsTestClient(t, p)
			_, err := c.GetAssistantBusinessFacts(context.Background(), agentSession, "available_models")
			if err == nil || p.authCalls != 1 || p.calls > 1 {
				t.Fatal(err, p.authCalls, p.calls)
			}
			if (mode == "api-key" || mode == "team" || mode == "unknown-actor") && p.calls != 0 {
				t.Fatal("invalid actor reached business RPC")
			}
		})
	}
}
