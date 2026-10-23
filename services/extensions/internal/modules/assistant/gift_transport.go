package assistant

import (
	"context"

	"github.com/TokenNotIncluded/lmm-api/services/extensions/internal/coreclient"
	pb "github.com/TokenNotIncluded/lmm-api/services/extensions/internal/corepb"
	"github.com/TokenNotIncluded/lmm-api/services/extensions/internal/modules/assistant/access"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
)

type PersonalGiftPort struct{ client *coreclient.AssistantClient }

var _ GiftPort = (*PersonalGiftPort)(nil)

func NewPersonalGiftPort(client *coreclient.AssistantClient) (*PersonalGiftPort, error) {
	if client == nil {
		return nil, ErrDelegationUnavailable
	}
	return &PersonalGiftPort{client: client}, nil
}

func giftRPCError(err error) error {
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
	case codes.NotFound:
		return access.ErrNotFound
	case codes.AlreadyExists, codes.Aborted:
		return access.ErrConflict
	case codes.Unimplemented, codes.FailedPrecondition:
		return ErrDelegationUnavailable
	default:
		return ErrGiftUnknown
	}
}

func decodeGiftReceipt(r *pb.AssistantGiftReceipt) (GiftReceipt, error) {
	if r == nil {
		return GiftReceipt{}, ErrGiftUnknown
	}
	value := GiftReceipt{ID: r.GetGiftId(), OwnerUserID: r.GetOwnerUserId(), CreditAmount: r.GetCreditAmount(), CreatedAt: r.GetCreatedAtUnix(), ClaimedAt: r.GetClaimedAtUnix(), LedgerTransactionID: r.GetLedgerTransactionId(), MaxCreditAmount: r.GetMaxCreditAmount(), ClaimAvailable: r.GetClaimAvailable(), ClaimBlockedCode: r.GetClaimBlockedCode()}
	switch r.GetState() {
	case pb.AssistantGiftState_ASSISTANT_GIFT_STATE_OFFERED:
		value.State = "offered"
	case pb.AssistantGiftState_ASSISTANT_GIFT_STATE_CLAIMED:
		value.State = "claimed"
	default:
		return GiftReceipt{}, ErrGiftUnknown
	}
	if _, err := checkedGift(value.OwnerUserID, value); err != nil {
		return GiftReceipt{}, ErrGiftUnknown
	}
	return value, nil
}

func (p *PersonalGiftPort) GetNewUserGift(ctx context.Context, credential string) (*GiftReceipt, error) {
	r, err := p.client.AssistantGetNewUserGift(ctx, credential)
	if err != nil {
		return nil, giftRPCError(err)
	}
	if r == nil {
		return nil, ErrGiftUnknown
	}
	if r.GetGift() == nil {
		return nil, nil
	}
	value, err := decodeGiftReceipt(r.GetGift())
	if err != nil {
		return nil, err
	}
	return &value, nil
}

func (p *PersonalGiftPort) ClaimStoredNewUserGift(ctx context.Context, credential string) (GiftReceipt, bool, error) {
	r, err := p.client.AssistantClaimStoredNewUserGift(ctx, credential)
	if err != nil {
		return GiftReceipt{}, false, giftRPCError(err)
	}
	if r == nil || r.GetGift() == nil {
		return GiftReceipt{}, false, ErrGiftUnknown
	}
	value, err := decodeGiftReceipt(r.GetGift())
	if err != nil || value.State != "claimed" {
		return GiftReceipt{}, false, ErrGiftUnknown
	}
	return value, r.GetAlreadyClaimed(), nil
}
