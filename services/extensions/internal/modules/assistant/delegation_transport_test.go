package assistant

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"net"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/coreclient"
	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/metadata"
	"google.golang.org/grpc/status"
)

func TestPersonalTextReceiptPreservesAuthoritativeOptionalAnswer(t *testing.T) {
	reservation := personalReservationFixture()
	binding, err := personalOperationBinding(reservation.Binding)
	if err != nil {
		t.Fatal(err)
	}
	answer, charged := "authoritative answer", int64(3)
	response := &pb.AssistantOperationReceipt{Binding: binding, RequestId: strings.Repeat("b", 64), State: pb.AssistantOperationState_ASSISTANT_OPERATION_STATE_SETTLED, ReservedCredits: 10, SettledCredits: &charged, AnswerText: &answer}
	out, err := personalOperationReceipt(reservation.Binding, response)
	if err != nil || out.AnswerText == nil || *out.AnswerText != answer {
		t.Fatal(out, err)
	}
	response.State, response.SettledCredits = pb.AssistantOperationState_ASSISTANT_OPERATION_STATE_RESERVED, nil
	if _, err := personalOperationReceipt(reservation.Binding, response); err != ErrDelegationUnknown {
		t.Fatal("nonterminal response supplied an authoritative answer", err)
	}
}

// This runs the actual protobuf codec, UDS client and metadata path against a
// controlled Go gRPC peer. It is not the Rust issuer, Session or ledger proof.
type assistantRPCFixture struct {
	pb.UnimplementedCoreControlServer
	pb.UnimplementedCoreAssistantModelServer
	mu           sync.Mutex
	feature      bool
	giftFeature  bool
	giftReads    int
	giftClaims   int
	giftMode     string
	major        uint32
	mode         string
	beginCalls   int
	reserveCalls int
	statusCalls  int
	entered      chan struct{}
}

type assistantRPCCounts struct{ begin, reserve, status int }

func (s *assistantRPCFixture) calls() assistantRPCCounts {
	s.mu.Lock()
	defer s.mu.Unlock()
	return assistantRPCCounts{s.beginCalls, s.reserveCalls, s.statusCalls}
}

const assistantControlFixture = "iiiiiiiiiiiiiiiiiiiiiiiiiiiiiiii"
const assistantHostFixture = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
const assistantSessionFixture = "ssssssssssssssssssssssssssssssss"

func (s *assistantRPCFixture) check(ctx context.Context, user bool) error {
	md, _ := metadata.FromIncomingContext(ctx)
	expectedHost := assistantControlFixture
	if user {
		expectedHost = assistantHostFixture
	}
	if v := md.Get("authorization"); len(v) != 1 || v[0] != "Bearer "+expectedHost {
		return status.Error(codes.Unauthenticated, "host metadata mismatch")
	}
	if v := md.Get("x-lmm-protocol"); len(v) != 1 || v[0] != "1" {
		return status.Error(codes.InvalidArgument, "protocol metadata mismatch")
	}
	v := md.Get("x-lmm-user-credential")
	if user && (len(v) != 1 || v[0] != assistantSessionFixture) || !user && len(v) != 0 {
		return status.Error(codes.Unauthenticated, "session metadata mismatch")
	}
	return nil
}
func (s *assistantRPCFixture) Capabilities(ctx context.Context, _ *pb.CapabilitiesRequest) (*pb.CapabilitiesResponse, error) {
	if err := s.check(ctx, false); err != nil {
		return nil, err
	}
	features := []string{}
	if s.giftFeature {
		features = append(features, coreclient.AssistantNewUserGiftFeature)
	}
	if s.feature {
		if s.mode != "execute-only" {
			features = append(features, coreclient.AssistantPersonalTextReserveFeature)
		}
		if s.mode != "admission-only" {
			features = append(features, coreclient.AssistantPersonalTextExecuteFeature)
		}
	}
	r := &pb.CapabilitiesResponse{ProtocolMajor: s.major, Features: features}
	if s.mode == "unknown-caps" {
		r.ProtoReflect().SetUnknown([]byte{0x98, 0x06, 0x01})
	}
	return r, nil
}
func (s *assistantRPCFixture) BeginConversation(ctx context.Context, r *pb.AssistantConversationKey) (*pb.AssistantConversationReceipt, error) {
	if err := s.check(ctx, true); err != nil {
		return nil, err
	}
	s.mu.Lock()
	s.beginCalls++
	s.mu.Unlock()
	return &pb.AssistantConversationReceipt{CreationKey: r.GetCreationKey(), CoreConversationId: "lmm_cv_" + strings.Repeat("c", 64)}, nil
}
func (s *assistantRPCFixture) ReserveText(ctx context.Context, r *pb.AssistantTextReservation) (*pb.AssistantOperationReceipt, error) {
	if err := s.check(ctx, true); err != nil {
		return nil, err
	}
	s.mu.Lock()
	s.reserveCalls++
	mode := s.mode
	s.mu.Unlock()
	switch mode {
	case "permission":
		return nil, status.Error(codes.PermissionDenied, "do-not-expose-provider-secret")
	case "resource":
		return nil, status.Error(codes.ResourceExhausted, "do-not-parse-error-text-as-budget")
	case "timeout":
		if s.entered != nil {
			close(s.entered)
		}
		<-ctx.Done()
		return nil, status.FromContextError(ctx.Err()).Err()
	}
	out := &pb.AssistantOperationReceipt{Binding: r.GetBinding(), RequestId: strings.Repeat("b", 64), State: pb.AssistantOperationState_ASSISTANT_OPERATION_STATE_RESERVED, ReservedCredits: 10}
	switch mode {
	case "unknown-enum":
		out.State = pb.AssistantOperationState(99)
	case "unknown-field":
		out.ProtoReflect().SetUnknown([]byte{0x98, 0x06, 0x01})
	case "oversized":
		out.RequestId = strings.Repeat("b", coreclient.MaxMessage)
	case "wrong-binding":
		out.Binding = &pb.AssistantOperationBinding{DelegationId: r.GetBinding().GetDelegationId(), CoreConversationId: r.GetBinding().GetCoreConversationId(), TurnId: "other", OperationKey: "other:0", TextSha256: r.GetBinding().GetTextSha256()}
	case "contradictory-credit":
		v := int64(1)
		out.SettledCredits = &v
	}
	return out, nil
}
func (s *assistantRPCFixture) GetOperation(ctx context.Context, r *pb.AssistantOperationBinding) (*pb.AssistantOperationReceipt, error) {
	if err := s.check(ctx, true); err != nil {
		return nil, err
	}
	s.mu.Lock()
	s.statusCalls++
	s.mu.Unlock()
	return &pb.AssistantOperationReceipt{Binding: r, RequestId: strings.Repeat("b", 64), State: pb.AssistantOperationState_ASSISTANT_OPERATION_STATE_UNKNOWN, ReservedCredits: 10}, nil
}

func newAssistantRPCFixture(t *testing.T, service *assistantRPCFixture) (*PersonalTextPort, *coreclient.AssistantClient) {
	t.Helper()
	dir, err := os.MkdirTemp(os.Getenv("LMM_ASSISTANT_TEST_SOCKET_DIR"), "a08-rpc-")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = os.RemoveAll(dir) })
	path := filepath.Join(dir, "core.sock")
	if len(path) > 100 {
		t.Fatal("provide a short private LMM_ASSISTANT_TEST_SOCKET_DIR fixture")
	}
	listener, err := net.Listen("unix", path)
	if err != nil {
		t.Fatal(err)
	}
	server := grpc.NewServer()
	pb.RegisterCoreControlServer(server, service)
	pb.RegisterCoreAssistantModelServer(server, service)
	go func() { _ = server.Serve(listener) }()
	t.Cleanup(server.Stop)
	control, err := coreclient.New(path, []byte(assistantControlFixture))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = control.Close() })
	client, err := coreclient.NewAssistantClient(path, []byte(assistantHostFixture), control)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = client.Close() })
	port, err := NewPersonalTextPort(client)
	if err != nil {
		t.Fatal(err)
	}
	return port, client
}
func personalReservationFixture() PersonalTextReservation {
	text := "中文 <exact bytes>"
	sum := sha256.Sum256([]byte(text))
	return PersonalTextReservation{Binding: PersonalTextBinding{ID: "lmm_ad_" + strings.Repeat("a", 64), CoreConversationID: "lmm_cv_" + strings.Repeat("c", 64), TurnID: "turn", Round: 0, OperationKey: "turn:0", TextSHA256: hex.EncodeToString(sum[:])}, Text: text}
}

func TestPersonalTextUDSReplacesHostSessionAndProtocolMetadata(t *testing.T) {
	s := &assistantRPCFixture{feature: true, major: 1}
	p, _ := newAssistantRPCFixture(t, s)
	ctx := metadata.NewOutgoingContext(context.Background(), metadata.Pairs("authorization", "evil", "authorization", "also-evil", "x-lmm-protocol", "999", "x-lmm-user-credential", "foreign"))
	registration, err := p.BeginConversation(ctx, assistantSessionFixture, "original-creation")
	if err != nil || registration.CreationKey != "original-creation" || s.calls().begin != 1 {
		t.Fatal("original metadata was not used")
	}
	operation, err := p.Reserve(ctx, assistantSessionFixture, personalReservationFixture())
	if err != nil || operation.Status != "reserved" || operation.ReservedCredits != 10 || s.calls().reserve != 1 {
		t.Fatal("typed reservation transport failed")
	}
	if _, ok := any(p).(Model); ok {
		t.Fatal("reservation-only port became a model reply adapter")
	}
}
func TestPersonalTextUDSRequiresMandatoryCapability(t *testing.T) {
	for _, name := range []string{"missing", "major", "unknown-caps", "admission-only", "execute-only"} {
		t.Run(name, func(t *testing.T) {
			s := &assistantRPCFixture{feature: true, major: 1}
			switch name {
			case "missing":
				s.feature = false
			case "major":
				s.major = 2
			case "unknown-caps":
				s.mode = name
			case "admission-only", "execute-only":
				s.mode = name
			}
			p, _ := newAssistantRPCFixture(t, s)
			if _, err := p.Reserve(context.Background(), assistantSessionFixture, personalReservationFixture()); !errors.Is(err, ErrDelegationUnavailable) {
				t.Fatal("unsupported capability was not unavailable")
			}
			if s.calls().reserve != 0 {
				t.Fatal("unsupported Core received a reservation")
			}
			if _, err := p.BeginConversation(context.Background(), assistantSessionFixture, strings.Repeat("a", 32)); !errors.Is(err, ErrDelegationUnavailable) || s.calls().begin != 0 {
				t.Fatal("unsupported Core received a conversation mutation", err)
			}
		})
	}
}

func TestGiftCapabilityCannotBeInferredFromPaidTextOrEnableTextByItself(t *testing.T) {
	peer := &assistantRPCFixture{feature: true, major: 1}
	_, client := newAssistantRPCFixture(t, peer)
	if status.Code(client.GiftReady(context.Background())) != codes.FailedPrecondition {
		t.Fatal("paid text implied gift capability")
	}
	peer = &assistantRPCFixture{giftFeature: true, major: 1}
	_, client = newAssistantRPCFixture(t, peer)
	if err := client.GiftReady(context.Background()); err != nil {
		t.Fatal(err)
	}
	if status.Code(client.Ready(context.Background())) != codes.FailedPrecondition {
		t.Fatal("gift capability enabled text execution")
	}
	if peer.calls() != (assistantRPCCounts{}) {
		t.Fatal("capability discovery mutated an operation")
	}
}
func TestPersonalTextUDSMalformedOrBoundedACKIsUnknownWithoutRetry(t *testing.T) {
	for _, mode := range []string{"unknown-enum", "unknown-field", "oversized", "wrong-binding", "contradictory-credit", "resource"} {
		t.Run(mode, func(t *testing.T) {
			s := &assistantRPCFixture{feature: true, major: 1, mode: mode}
			p, _ := newAssistantRPCFixture(t, s)
			reservation := personalReservationFixture()
			if _, err := p.Reserve(context.Background(), assistantSessionFixture, reservation); !errors.Is(err, ErrDelegationUnknown) {
				t.Fatal("possibly committed invalid ACK was not unknown")
			}
			status, err := p.Status(context.Background(), assistantSessionFixture, reservation.Binding)
			if err != nil || status.Status != "unknown" || s.calls().reserve != 1 || s.calls().status != 1 {
				t.Fatal("uncertainty repeated ReserveText instead of original GetOperation")
			}
		})
	}
}
func TestPersonalTextUDSClosedDenialAndTimeout(t *testing.T) {
	for _, mode := range []string{"permission", "timeout"} {
		t.Run(mode, func(t *testing.T) {
			s := &assistantRPCFixture{feature: true, major: 1, mode: mode, entered: make(chan struct{})}
			p, _ := newAssistantRPCFixture(t, s)
			ctx, cancel := context.WithCancel(context.Background())
			defer cancel()
			result := make(chan error, 1)
			go func() { _, err := p.Reserve(ctx, assistantSessionFixture, personalReservationFixture()); result <- err }()
			if mode == "timeout" {
				select {
				case <-s.entered:
					cancel()
				case <-time.After(3 * time.Second):
					t.Fatal("reservation did not enter the server")
				}
			}
			var err error
			select {
			case err = <-result:
			case <-time.After(3 * time.Second):
				t.Fatal("reservation did not honor cancellation")
			}
			expected := access.ErrForbidden
			if mode == "timeout" {
				expected = ErrDelegationUnknown
			}
			if err != expected || s.calls().reserve != 1 {
				t.Fatal("RPC error was exposed, retried or misclassified")
			}
		})
	}
}
func TestPersonalTextUDSRejectsUnknownRequestFieldsBeforeRPC(t *testing.T) {
	s := &assistantRPCFixture{feature: true, major: 1}
	_, client := newAssistantRPCFixture(t, s)
	request := &pb.AssistantConversationKey{CreationKey: "original"}
	request.ProtoReflect().SetUnknown([]byte{0x98, 0x06, 0x01})
	if _, err := client.AssistantBeginConversation(context.Background(), assistantSessionFixture, request); status.Code(err) != codes.InvalidArgument || s.calls().begin != 0 {
		t.Fatal("unknown request authority reached Core")
	}
}

func TestPersonalTextDedicatedCredentialCannotReuseControlClient(t *testing.T) {
	control, err := coreclient.New("/tmp/lmm-unmounted-assistant-test.sock", []byte(assistantControlFixture))
	if err != nil {
		t.Fatal(err)
	}
	defer control.Close()
	if client, err := coreclient.NewAssistantClient("/tmp/lmm-unmounted-assistant-test.sock", []byte(assistantControlFixture), control); err == nil || client != nil {
		t.Fatal("identity/control token became the assistant credential")
	}
	if client, err := coreclient.NewAssistantClient("/tmp/lmm-unmounted-assistant-test.sock", []byte(assistantHostFixture), nil); err == nil || client != nil {
		t.Fatal("missing read-only capability discovery was accepted")
	}
	port, err := NewPersonalTextPort(&coreclient.AssistantClient{})
	if err != nil {
		t.Fatal(err)
	}
	if _, err = port.Reserve(context.Background(), assistantSessionFixture, personalReservationFixture()); !errors.Is(err, ErrDelegationUnavailable) {
		t.Fatal("zero/unconfigured client did not fail closed")
	}
}
