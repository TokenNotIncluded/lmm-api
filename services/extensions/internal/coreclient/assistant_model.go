package coreclient

import (
	"bytes"
	"context"
	"crypto/sha256"
	"errors"
	"strconv"
	"strings"
	"unicode"
	"unicode/utf8"

	pb "github.com/TokenNotIncluded/api.lmm.best/extensions/internal/corepb"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
	"google.golang.org/protobuf/proto"
	"google.golang.org/protobuf/reflect/protoreflect"
)

const AssistantPersonalTextReserveFeature = "assistant.personal_text.reserve.v1"
const AssistantPersonalTextExecuteFeature = "assistant.personal_text.execute.v1"
const AssistantNewUserGiftFeature = "assistant.new_user_gift.v1"

// AssistantClient separates read-only capability discovery from the actual
// assistant credential. Its only public calls are this narrow assistant port.
// Core must still install and authorize that scope; local separation cannot
// invent a server permission or an issuer.
type AssistantClient struct {
	rpc     *Client
	control *Client
}

func NewAssistantClient(socket string, credential []byte, control *Client) (*AssistantClient, error) {
	if control == nil || control.connection == nil {
		return nil, errors.New("assistant capability control client required")
	}
	rpc, err := New(socket, credential)
	if err != nil {
		return nil, err
	}
	if rpc.authorization == control.authorization {
		_ = rpc.Close()
		return nil, errors.New("assistant RPC credential must be independent")
	}
	return &AssistantClient{rpc: rpc, control: control}, nil
}
func (c *AssistantClient) Close() error {
	if c == nil || c.rpc == nil {
		return nil
	}
	return c.rpc.Close()
}

// The new port uses the existing bounded UDS connection, admission/deadline and
// metadata replacement. A dedicated module host credential is still required at
// composition; possessing it never supplies the user or authority to spend.
func (c *AssistantClient) assistantReady(ctx context.Context, user string) error {
	if !validUserCredential(user) {
		return status.Error(codes.Unauthenticated, "invalid user credential")
	}
	return c.Ready(ctx)
}

// Ready discovers only the exact mandatory capability through the control
// credential; it does not send a Session or authorize an assistant mutation.
func (c *AssistantClient) Ready(ctx context.Context) error {
	// Admission alone cannot provide a billed answer. Both are mandatory.
	return c.assistantFeatures(ctx, AssistantPersonalTextReserveFeature, AssistantPersonalTextExecuteFeature)
}

// GiftReady is independent of text execution. It is advertised only when Core
// has installed the gift policy, trusted decision hook and durable ledger path.
func (c *AssistantClient) GiftReady(ctx context.Context) error {
	return c.assistantFeatures(ctx, AssistantNewUserGiftFeature)
}

func (c *AssistantClient) assistantFeatures(ctx context.Context, required ...string) error {
	if c == nil || c.rpc == nil || c.control == nil {
		return status.Error(codes.FailedPrecondition, "assistant client unavailable")
	}
	capabilities, err := c.control.Capabilities(ctx)
	if err != nil {
		// This is a preflight, so no assistant mutation was sent. Never fall
		// back to the legacy JSON bridge or an API/root key on an old Core.
		return status.Error(codes.FailedPrecondition, "assistant capability unavailable")
	}
	if !assistantMessage(capabilities) || capabilities.GetProtocolMajor() != 1 || len(capabilities.GetFeatures()) > 128 {
		return status.Error(codes.FailedPrecondition, "assistant capability unavailable")
	}
	seen := map[string]bool{}
	for _, feature := range capabilities.GetFeatures() {
		if !assistantLabel(feature, 256) || seen[feature] {
			return status.Error(codes.FailedPrecondition, "assistant capability unavailable")
		}
		seen[feature] = true
	}
	for _, feature := range required {
		if !seen[feature] {
			return status.Error(codes.FailedPrecondition, "assistant capability unavailable")
		}
	}
	return nil
}

func assistantLabel(v string, max int) bool {
	return v != "" && len(v) <= max && utf8.ValidString(v) && !strings.ContainsFunc(v, func(r rune) bool { return unicode.IsControl(r) || unicode.IsSpace(r) })
}

func assistantDigest(v string, prefix string) bool {
	if !strings.HasPrefix(v, prefix) || len(v) != len(prefix)+64 {
		return false
	}
	for _, ch := range v[len(prefix):] {
		if !(ch >= '0' && ch <= '9' || ch >= 'a' && ch <= 'f') {
			return false
		}
	}
	return true
}

// Strict only for this new mandatory capability; existing protobuf consumers
// retain their original compatibility rules. Size is also enforced by gRPC.
func assistantMessage(message proto.Message) bool {
	if message == nil || !message.ProtoReflect().IsValid() || proto.Size(message) > MaxMessage {
		return false
	}
	var valid func(protoreflect.Message) bool
	valid = func(m protoreflect.Message) bool {
		if len(m.GetUnknown()) != 0 {
			return false
		}
		ok := true
		m.Range(func(field protoreflect.FieldDescriptor, value protoreflect.Value) bool {
			if field.IsMap() {
				ok = false
			} else if field.IsList() {
				if field.Kind() == protoreflect.MessageKind {
					for i := 0; i < value.List().Len(); i++ {
						if !valid(value.List().Get(i).Message()) {
							ok = false
							return false
						}
					}
				}
			} else if field.Kind() == protoreflect.MessageKind {
				ok = valid(value.Message())
			} else if field.Kind() == protoreflect.EnumKind {
				ok = field.Enum().Values().ByNumber(value.Enum()) != nil
			}
			return ok
		})
		return ok
	}
	return valid(message.ProtoReflect())
}

func assistantReply(message proto.Message, err error) error {
	if err != nil {
		return err
	}
	if !assistantMessage(message) {
		return status.Error(codes.DataLoss, "invalid assistant response")
	}
	return nil
}

func assistantBinding(r *pb.AssistantOperationBinding) bool {
	return r != nil && assistantMessage(r) && assistantDigest(r.GetDelegationId(), "lmm_ad_") && assistantDigest(r.GetCoreConversationId(), "lmm_cv_") && assistantLabel(r.GetTurnId(), 124) && r.GetRound() < 4 && r.GetOperationKey() == r.GetTurnId()+":"+strconv.Itoa(int(r.GetRound())) && len(r.GetTextSha256()) == sha256.Size
}

func (c *AssistantClient) AssistantBeginConversation(ctx context.Context, user string, r *pb.AssistantConversationKey) (*pb.AssistantConversationReceipt, error) {
	if r == nil || !assistantMessage(r) || !assistantLabel(r.GetCreationKey(), 128) {
		return nil, status.Error(codes.InvalidArgument, "invalid assistant request")
	}
	if err := c.assistantReady(ctx, user); err != nil {
		return nil, err
	}
	reply, err := pb.NewCoreAssistantModelClient(c.rpc.connection).BeginConversation(c.rpc.context(ctx, user), r)
	return reply, assistantReply(reply, err)
}
func (c *AssistantClient) AssistantLookupConversation(ctx context.Context, user string, r *pb.AssistantConversationKey) (*pb.AssistantConversationReceipt, error) {
	if r == nil || !assistantMessage(r) || !assistantLabel(r.GetCreationKey(), 128) {
		return nil, status.Error(codes.InvalidArgument, "invalid assistant request")
	}
	if err := c.assistantReady(ctx, user); err != nil {
		return nil, err
	}
	reply, err := pb.NewCoreAssistantModelClient(c.rpc.connection).LookupConversation(c.rpc.context(ctx, user), r)
	return reply, assistantReply(reply, err)
}
func assistantTurnKey(r *pb.AssistantTurnKey) bool {
	return r != nil && assistantMessage(r) && assistantDigest(r.GetCoreConversationId(), "lmm_cv_") && assistantLabel(r.GetTurnId(), 124)
}
func (c *AssistantClient) AssistantIssueGrant(ctx context.Context, user string, r *pb.AssistantTurnKey) (*pb.AssistantGrantReceipt, error) {
	if !assistantTurnKey(r) {
		return nil, status.Error(codes.InvalidArgument, "invalid assistant request")
	}
	if err := c.assistantReady(ctx, user); err != nil {
		return nil, err
	}
	reply, err := pb.NewCoreAssistantModelClient(c.rpc.connection).IssueGrant(c.rpc.context(ctx, user), r)
	return reply, assistantReply(reply, err)
}
func (c *AssistantClient) AssistantLookupGrant(ctx context.Context, user string, r *pb.AssistantTurnKey) (*pb.AssistantGrantReceipt, error) {
	if !assistantTurnKey(r) {
		return nil, status.Error(codes.InvalidArgument, "invalid assistant request")
	}
	if err := c.assistantReady(ctx, user); err != nil {
		return nil, err
	}
	reply, err := pb.NewCoreAssistantModelClient(c.rpc.connection).LookupGrant(c.rpc.context(ctx, user), r)
	return reply, assistantReply(reply, err)
}
func (c *AssistantClient) AssistantReserveText(ctx context.Context, user string, r *pb.AssistantTextReservation) (*pb.AssistantOperationReceipt, error) {
	if r == nil || !assistantMessage(r) || !assistantBinding(r.GetBinding()) || !utf8.ValidString(r.GetText()) || len(r.GetText()) == 0 || len(r.GetText()) > 8192 {
		return nil, status.Error(codes.InvalidArgument, "invalid assistant request")
	}
	digest := sha256.Sum256([]byte(r.GetText()))
	if !bytes.Equal(digest[:], r.GetBinding().GetTextSha256()) {
		return nil, status.Error(codes.InvalidArgument, "invalid assistant request")
	}
	if err := c.assistantReady(ctx, user); err != nil {
		return nil, err
	}
	reply, err := pb.NewCoreAssistantModelClient(c.rpc.connection).ReserveText(c.rpc.context(ctx, user), r)
	return reply, assistantReply(reply, err)
}
func (c *AssistantClient) AssistantGetOperation(ctx context.Context, user string, r *pb.AssistantOperationBinding) (*pb.AssistantOperationReceipt, error) {
	if !assistantBinding(r) {
		return nil, status.Error(codes.InvalidArgument, "invalid assistant request")
	}
	if err := c.assistantReady(ctx, user); err != nil {
		return nil, err
	}
	reply, err := pb.NewCoreAssistantModelClient(c.rpc.connection).GetOperation(c.rpc.context(ctx, user), r)
	return reply, assistantReply(reply, err)
}
func (c *AssistantClient) AssistantRevokeGrant(ctx context.Context, user string, r *pb.AssistantRevokeGrant) (*pb.AssistantRevokeReceipt, error) {
	if r == nil || !assistantMessage(r) || !assistantDigest(r.GetDelegationId(), "lmm_ad_") {
		return nil, status.Error(codes.InvalidArgument, "invalid assistant request")
	}
	if err := c.assistantReady(ctx, user); err != nil {
		return nil, err
	}
	reply, err := pb.NewCoreAssistantModelClient(c.rpc.connection).RevokeGrant(c.rpc.context(ctx, user), r)
	return reply, assistantReply(reply, err)
}
