package assistant

import (
	"context"
	"encoding/hex"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/coreclient"
	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
)

// PersonalTextPort uses the real Core client UDS/protobuf path. It exposes no
// URLs, provider keys, actor/payer flags, worker lease or Model.Next method.
// Capabilities are checked per call by coreclient before any new RPC is sent.
type PersonalTextPort struct{ client *coreclient.AssistantClient }

var _ DurableDelegationPort = (*PersonalTextPort)(nil)

func NewPersonalTextPort(client *coreclient.AssistantClient) (*PersonalTextPort, error) {
	if client == nil {
		return nil, ErrDelegationUnavailable
	}
	return &PersonalTextPort{client: client}, nil
}

func personalTextRPCError(err error) error {
	if err == nil {
		return nil
	}
	switch status.Code(err) {
	case codes.InvalidArgument, codes.OutOfRange:
		return access.ErrInvalid
	case codes.Unauthenticated:
		return access.ErrUnauthorized
	case codes.PermissionDenied:
		return access.ErrForbidden
	case codes.AlreadyExists, codes.Aborted:
		return access.ErrConflict
	case codes.NotFound:
		return access.ErrNotFound
	case codes.Unimplemented, codes.FailedPrecondition:
		return ErrDelegationUnavailable
	default:
		// ResourceExhausted can be a local receive bound AFTER Core committed.
		// Budget/no-funds need a future root-reviewed typed rejection reason;
		// never parse arbitrary error text or blindly reserve another operation.
		return ErrDelegationUnknown
	}
}

func (p *PersonalTextPort) BeginConversation(ctx context.Context, credential, key string) (CoreConversationRegistration, error) {
	if !delegationLabel(key, 128) {
		return CoreConversationRegistration{}, access.ErrInvalid
	}
	r, err := p.client.AssistantBeginConversation(ctx, credential, &pb.AssistantConversationKey{CreationKey: key})
	if err != nil {
		return CoreConversationRegistration{}, personalTextRPCError(err)
	}
	return personalConversationReceipt(key, r)
}
func (p *PersonalTextPort) LookupConversation(ctx context.Context, credential, key string) (CoreConversationRegistration, error) {
	if !delegationLabel(key, 128) {
		return CoreConversationRegistration{}, access.ErrInvalid
	}
	r, err := p.client.AssistantLookupConversation(ctx, credential, &pb.AssistantConversationKey{CreationKey: key})
	if err != nil {
		return CoreConversationRegistration{}, personalTextRPCError(err)
	}
	return personalConversationReceipt(key, r)
}
func personalConversationReceipt(key string, r *pb.AssistantConversationReceipt) (CoreConversationRegistration, error) {
	if r == nil || r.GetCreationKey() != key || !coreConversationID(r.GetCoreConversationId()) {
		return CoreConversationRegistration{}, ErrDelegationUnknown
	}
	return CoreConversationRegistration{CreationKey: key, ID: r.GetCoreConversationId()}, nil
}
func personalTurnKey(key PersonalTextTurnKey) (*pb.AssistantTurnKey, error) {
	if !coreConversationID(key.CoreConversationID) || !delegationLabel(key.TurnID, 124) {
		return nil, access.ErrInvalid
	}
	return &pb.AssistantTurnKey{CoreConversationId: key.CoreConversationID, TurnId: key.TurnID}, nil
}
func (p *PersonalTextPort) Issue(ctx context.Context, credential string, key PersonalTextTurnKey) (PersonalTextGrant, error) {
	r, err := personalTurnKey(key)
	if err != nil {
		return PersonalTextGrant{}, err
	}
	receipt, err := p.client.AssistantIssueGrant(ctx, credential, r)
	if err != nil {
		return PersonalTextGrant{}, personalTextRPCError(err)
	}
	return personalGrantReceipt(key, receipt)
}
func (p *PersonalTextPort) LookupIssue(ctx context.Context, credential string, key PersonalTextTurnKey) (PersonalTextGrant, error) {
	r, err := personalTurnKey(key)
	if err != nil {
		return PersonalTextGrant{}, err
	}
	receipt, err := p.client.AssistantLookupGrant(ctx, credential, r)
	if err != nil {
		return PersonalTextGrant{}, personalTextRPCError(err)
	}
	return personalGrantReceipt(key, receipt)
}
func personalGrantReceipt(key PersonalTextTurnKey, r *pb.AssistantGrantReceipt) (PersonalTextGrant, error) {
	if r == nil {
		return PersonalTextGrant{}, ErrDelegationUnknown
	}
	g := PersonalTextGrant{ID: r.GetDelegationId(), CoreConversationID: r.GetCoreConversationId(), TurnID: r.GetTurnId(), ExpiresAt: r.GetExpiresAtUnix(), MaxModelCalls: int(r.GetMaxModelCalls()), Revoked: r.GetRevoked()}
	if !validPersonalTextGrant(g) || g.CoreConversationID != key.CoreConversationID || g.TurnID != key.TurnID {
		return PersonalTextGrant{}, ErrDelegationUnknown
	}
	return g, nil
}
func personalOperationBinding(binding PersonalTextBinding) (*pb.AssistantOperationBinding, error) {
	if !validPersonalTextBinding(binding) {
		return nil, access.ErrInvalid
	}
	digest, err := hex.DecodeString(binding.TextSHA256)
	if err != nil {
		return nil, access.ErrInvalid
	}
	return &pb.AssistantOperationBinding{DelegationId: binding.ID, CoreConversationId: binding.CoreConversationID, TurnId: binding.TurnID, Round: uint32(binding.Round), OperationKey: binding.OperationKey, TextSha256: digest}, nil
}
func (p *PersonalTextPort) Reserve(ctx context.Context, credential string, reservation PersonalTextReservation) (PersonalTextOperation, error) {
	if !validPersonalTextReservation(reservation) {
		return PersonalTextOperation{}, access.ErrInvalid
	}
	binding, err := personalOperationBinding(reservation.Binding)
	if err != nil {
		return PersonalTextOperation{}, err
	}
	receipt, err := p.client.AssistantReserveText(ctx, credential, &pb.AssistantTextReservation{Binding: binding, Text: reservation.Text})
	if err != nil {
		return PersonalTextOperation{}, personalTextRPCError(err)
	}
	return personalOperationReceipt(reservation.Binding, receipt)
}
func (p *PersonalTextPort) Status(ctx context.Context, credential string, binding PersonalTextBinding) (PersonalTextOperation, error) {
	r, err := personalOperationBinding(binding)
	if err != nil {
		return PersonalTextOperation{}, err
	}
	receipt, err := p.client.AssistantGetOperation(ctx, credential, r)
	if err != nil {
		return PersonalTextOperation{}, personalTextRPCError(err)
	}
	return personalOperationReceipt(binding, receipt)
}
func personalOperationReceipt(expected PersonalTextBinding, r *pb.AssistantOperationReceipt) (PersonalTextOperation, error) {
	if r == nil || r.GetBinding() == nil || len(r.GetBinding().GetTextSha256()) != 32 {
		return PersonalTextOperation{}, ErrDelegationUnknown
	}
	b := r.GetBinding()
	value := PersonalTextOperation{Binding: PersonalTextBinding{ID: b.GetDelegationId(), CoreConversationID: b.GetCoreConversationId(), TurnID: b.GetTurnId(), Round: int(b.GetRound()), OperationKey: b.GetOperationKey(), TextSHA256: hex.EncodeToString(b.GetTextSha256())}, RequestID: r.GetRequestId(), ReservedCredits: r.GetReservedCredits(), SettledCredits: r.SettledCredits, AnswerText: r.AnswerText}
	if r.GetGiftReceipt() != nil {
		gift, err := decodeGiftReceipt(r.GetGiftReceipt())
		if err != nil {
			return PersonalTextOperation{}, ErrDelegationUnknown
		}
		value.GiftReceipt = &gift
	}
	switch r.GetState() {
	case pb.AssistantOperationState_ASSISTANT_OPERATION_STATE_RESERVED:
		value.Status = "reserved"
	case pb.AssistantOperationState_ASSISTANT_OPERATION_STATE_STREAMING:
		value.Status = "streaming"
	case pb.AssistantOperationState_ASSISTANT_OPERATION_STATE_SETTLED:
		value.Status = "settled"
	case pb.AssistantOperationState_ASSISTANT_OPERATION_STATE_RELEASED:
		value.Status = "released"
	case pb.AssistantOperationState_ASSISTANT_OPERATION_STATE_UNKNOWN:
		value.Status = "unknown"
	default:
		return PersonalTextOperation{}, ErrDelegationUnknown
	}
	checked, err := checkedPersonalTextOperation(expected, value, nil)
	if err != nil {
		return PersonalTextOperation{}, ErrDelegationUnknown
	}
	return checked, nil
}
func (p *PersonalTextPort) Revoke(ctx context.Context, credential, grantID string) error {
	if !delegationID(grantID) {
		return access.ErrInvalid
	}
	r, err := p.client.AssistantRevokeGrant(ctx, credential, &pb.AssistantRevokeGrant{DelegationId: grantID})
	if err != nil {
		return personalTextRPCError(err)
	}
	if r == nil || r.GetDelegationId() != grantID || !r.GetRevoked() {
		return ErrDelegationUnknown
	}
	return nil
}
