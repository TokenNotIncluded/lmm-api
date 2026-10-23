package coreclient

import (
	"context"
	"crypto/sha256"
	"net"
	"os"
	"path/filepath"
	"strings"
	"testing"

	pb "github.com/TokenNotIncluded/api.lmm.best/extensions/internal/corepb"
	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/metadata"
	"google.golang.org/grpc/status"
)

type confirmationPeer struct {
	agentPeer
	enabled bool
}

func (p *confirmationPeer) Capabilities(ctx context.Context, _ *pb.CapabilitiesRequest) (*pb.CapabilitiesResponse, error) {
	md, _ := metadata.FromIncomingContext(ctx)
	if len(md.Get("x-lmm-user-credential")) != 0 {
		return nil, status.Error(codes.Unauthenticated, "user in preflight")
	}
	features := []string{AssistantAgentTurnFeature}
	if p.enabled {
		features = []string{AssistantToolConfirmationFeature}
	}
	return &pb.CapabilitiesResponse{ProtocolMajor: 1, Features: features}, nil
}
func confirmationFixture(operation string) *pb.AssistantToolConfirmation {
	preview := []byte(`{"amount":9007199254740993,"text":"<hello>"}`)
	hash := sha256.Sum256(preview)
	return &pb.AssistantToolConfirmation{ChallengeId: strings.Repeat("a", 64), ToolOperationId: operation, ToolName: "create_site_issue", ToolVersion: "v1", SourceArgumentsSha256: make([]byte, 32), NormalizedDraftSha256: hash[:], PreviewJson: preview, ExpiresAtUnix: 123456, State: "pending", BrowserPath: "/api/user/assistant/confirmations/" + strings.Repeat("a", 64)}
}
func (p *confirmationPeer) reply(operation string, receipt bool) (*pb.AssistantToolConfirmation, error) {
	r := confirmationFixture(operation)
	if receipt {
		r.State = "approved"
	}
	switch p.mode {
	case "lost":
		return nil, status.Error(codes.Unavailable, "private detail")
	case "operation":
		r.ToolOperationId = "other"
	case "digest":
		r.NormalizedDraftSha256 = make([]byte, 32)
	case "source":
		r.SourceArgumentsSha256 = nil
	case "unknown":
		r.ProtoReflect().SetUnknown([]byte{0x98, 0x06, 1})
	case "executed":
		r.Executed = true
	case "state":
		r.State = "invented"
	case "path":
		r.BrowserPath = "https://evil.invalid/"
	case "pending":
		r.State = "pending"
	case "preview":
		r.PreviewJson = []byte(`{"other":true}`)
		h := sha256.Sum256(r.PreviewJson)
		r.NormalizedDraftSha256 = h[:]
	}
	return r, nil
}
func (p *confirmationPeer) PrepareAssistantToolConfirmation(ctx context.Context, r *pb.PrepareAssistantToolConfirmationRequest) (*pb.AssistantToolConfirmation, error) {
	if err := p.check(ctx); err != nil {
		return nil, err
	}
	return p.reply(r.ToolOperationId, false)
}
func (p *confirmationPeer) GetAssistantToolConfirmationReceipt(ctx context.Context, r *pb.GetAssistantToolConfirmationReceiptRequest) (*pb.AssistantToolConfirmation, error) {
	if err := p.check(ctx); err != nil {
		return nil, err
	}
	return p.reply(r.ToolOperationId, true)
}
func confirmationTestClient(t *testing.T, p *confirmationPeer) *AssistantClient {
	t.Helper()
	dir, err := os.MkdirTemp("", "confirm-uds-")
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
func TestConfirmationIndependentCapabilityAndOriginalSession(t *testing.T) {
	p := &confirmationPeer{}
	c := confirmationTestClient(t, p)
	draft := []byte(`{ "text":"\u003chello\u003e", "amount":9007199254740993 }`)
	r := &pb.PrepareAssistantToolConfirmationRequest{ToolOperationId: "operation", NormalizedDraftJson: draft}
	ctx := metadata.NewOutgoingContext(context.Background(), metadata.Pairs("authorization", "evil", "x-lmm-user-credential", "evil"))
	if _, err := c.PrepareAssistantToolConfirmation(ctx, agentSession, r); status.Code(err) != codes.FailedPrecondition || p.calls != 0 {
		t.Fatal(err, p.calls)
	}
	p = &confirmationPeer{enabled: true}
	c = confirmationTestClient(t, p)
	out, err := c.PrepareAssistantToolConfirmation(ctx, agentSession, r)
	if err != nil {
		t.Fatal(err)
	}
	if _, err = c.GetAssistantToolConfirmationReceipt(ctx, agentSession, &pb.GetAssistantToolConfirmationReceiptRequest{ToolOperationId: "operation", NormalizedDraftSha256: out.NormalizedDraftSha256}); err != nil || p.calls != 2 {
		t.Fatal(err, p.calls)
	}
	if c.AgentReady(ctx) == nil || c.Ready(ctx) == nil {
		t.Fatal("confirmation implied agent or paid text authority")
	}
}
func TestConfirmationCorruptAcknowledgementNeverRetries(t *testing.T) {
	for _, mode := range []string{"lost", "operation", "digest", "source", "unknown", "executed", "state", "path", "preview"} {
		t.Run(mode, func(t *testing.T) {
			p := &confirmationPeer{enabled: true}
			p.mode = mode
			c := confirmationTestClient(t, p)
			_, err := c.PrepareAssistantToolConfirmation(context.Background(), agentSession, &pb.PrepareAssistantToolConfirmationRequest{ToolOperationId: "operation", NormalizedDraftJson: confirmationFixture("operation").PreviewJson})
			if err == nil || p.calls != 1 {
				t.Fatal(err, p.calls)
			}
		})
	}
	for _, mode := range []string{"pending", "preview", "digest", "executed", "unknown"} {
		t.Run("receipt-"+mode, func(t *testing.T) {
			p := &confirmationPeer{enabled: true}
			p.mode = mode
			c := confirmationTestClient(t, p)
			_, err := c.GetAssistantToolConfirmationReceipt(context.Background(), agentSession, &pb.GetAssistantToolConfirmationReceiptRequest{ToolOperationId: "operation", NormalizedDraftSha256: confirmationFixture("operation").NormalizedDraftSha256})
			if err == nil || p.calls != 1 {
				t.Fatal(err, p.calls)
			}
		})
	}
}
func TestConfirmationRejectsNonObjectsDuplicateKeysAndAuthorityFields(t *testing.T) {
	p := &confirmationPeer{enabled: true}
	c := confirmationTestClient(t, p)
	for _, draft := range []string{`[]`, `null`, `{"a":1,"a":2}`, `{"nested":{"a":1,"a":2}}`, `{} {}`, `{"x":"` + strings.Repeat("a", maxConfirmationDraft) + `"}`} {
		if _, err := c.PrepareAssistantToolConfirmation(context.Background(), agentSession, &pb.PrepareAssistantToolConfirmationRequest{ToolOperationId: "operation", NormalizedDraftJson: []byte(draft)}); status.Code(err) != codes.InvalidArgument || p.calls != 0 {
			t.Fatal(err, p.calls)
		}
	}
	r := &pb.PrepareAssistantToolConfirmationRequest{ToolOperationId: "operation", NormalizedDraftJson: []byte(`{}`)}
	r.ProtoReflect().SetUnknown([]byte{0x98, 0x06, 1})
	if _, err := c.PrepareAssistantToolConfirmation(context.Background(), agentSession, r); status.Code(err) != codes.InvalidArgument || p.calls != 0 {
		t.Fatal(err, p.calls)
	}
}
