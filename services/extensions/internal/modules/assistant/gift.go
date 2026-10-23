package assistant

import (
	"context"
	"errors"
	"strconv"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
)

var ErrGiftUnknown = errors.New("assistant_gift_unknown")

// GiftReceipt comes only from Core's original Session-authenticated gift ledger.
// This port cannot choose a recipient, amount, payer, decision or eligibility.
type GiftReceipt struct {
	ID                  string
	OwnerUserID         int64
	State               string
	CreditAmount        int64
	CreatedAt           int64
	ClaimedAt           int64
	LedgerTransactionID string
	MaxCreditAmount     int64
	ClaimAvailable      bool
	ClaimBlockedCode    string
}

type GiftPort interface {
	GetNewUserGift(context.Context, string) (*GiftReceipt, error)
	ClaimStoredNewUserGift(context.Context, string) (GiftReceipt, bool, error)
}

// NewUserGift preserves the Web receipt shape. Legacy cents are always zero;
// the stored integer ledger grant is authoritative. No fiat value is inferred.
type NewUserGift struct {
	ID               string   `json:"id"`
	Status           string   `json:"status"`
	CreditAmount     int64    `json:"credit_amount"`
	Quota            int64    `json:"quota"`
	CreatedAt        int64    `json:"created_at"`
	ClaimedAt        int64    `json:"claimed_at"`
	Reason           string   `json:"reason"`
	AmountCents      int64    `json:"amount_cents"`
	AmountUnit       string   `json:"amount_unit"`
	AmountUSD        *float64 `json:"amount_usd"`
	Currency         string   `json:"currency"`
	CreditsPerUSD    *float64 `json:"credits_per_usd"`
	MaxCreditAmount  int64    `json:"max_credit_amount"`
	ClaimAvailable   bool     `json:"claim_available"`
	ClaimBlockedCode string   `json:"claim_blocked_code,omitempty"`
}

type GiftClaim struct {
	Gift           NewUserGift `json:"gift"`
	AlreadyClaimed bool        `json:"already_claimed"`
}

type Gifts struct {
	authority access.Authority
	port      GiftPort
}

func NewGifts(authority access.Authority, port GiftPort) (*Gifts, error) {
	if access.Missing(authority) || access.Missing(port) {
		return nil, ErrDelegationUnavailable
	}
	return &Gifts{authority: authority, port: port}, nil
}

func (s *Gifts) owner(ctx context.Context, credential string) (int64, error) {
	p, err := s.authority.Check(ctx, credential, access.Account{}, access.Read)
	if err != nil {
		return 0, err
	}
	if p.UserID <= 0 || p.Level < 0 || p.Level > 6 || p.Account != (access.Account{Kind: "personal", ID: p.UserID}) {
		return 0, access.ErrForbidden
	}
	return p.UserID, nil
}

func checkedGift(owner int64, r GiftReceipt) (NewUserGift, error) {
	if owner <= 0 || !lowerDigest(r.ID) || r.OwnerUserID != owner || r.CreditAmount <= 0 || r.CreditAmount > maxSafeHistoryID || r.CreatedAt <= 0 || r.MaxCreditAmount < 0 || r.MaxCreditAmount > maxSafeHistoryID {
		return NewUserGift{}, ErrGiftUnknown
	}
	switch r.State {
	case "offered":
		if r.ClaimedAt != 0 || r.LedgerTransactionID != "" {
			return NewUserGift{}, ErrGiftUnknown
		}
		if r.ClaimAvailable {
			if r.ClaimBlockedCode != "" || r.MaxCreditAmount <= 0 || r.CreditAmount >= r.MaxCreditAmount {
				return NewUserGift{}, ErrGiftUnknown
			}
		} else {
			switch r.ClaimBlockedCode {
			case "disabled", "cap_changed", "qualification_missing", "registration_hold", "reward_used", "identity_used", "network_limit":
			default:
				return NewUserGift{}, ErrGiftUnknown
			}
		}
	case "claimed":
		journal, err := strconv.ParseInt(r.LedgerTransactionID, 10, 64)
		if err != nil || journal <= 0 || strconv.FormatInt(journal, 10) != r.LedgerTransactionID || r.ClaimedAt < r.CreatedAt || r.ClaimAvailable || r.ClaimBlockedCode != "already_claimed" {
			return NewUserGift{}, ErrGiftUnknown
		}
	default:
		return NewUserGift{}, ErrGiftUnknown
	}
	return NewUserGift{ID: r.ID, Status: r.State, CreditAmount: r.CreditAmount, Quota: r.CreditAmount, CreatedAt: r.CreatedAt, ClaimedAt: r.ClaimedAt, Reason: "Welcome gift", AmountUnit: "LEGACY_CENTS", Currency: "USD", MaxCreditAmount: r.MaxCreditAmount, ClaimAvailable: r.ClaimAvailable, ClaimBlockedCode: r.ClaimBlockedCode}, nil
}

func (s *Gifts) GetNewUserGift(ctx context.Context, credential string) (*NewUserGift, error) {
	owner, err := s.owner(ctx, credential)
	if err != nil {
		return nil, err
	}
	r, err := s.port.GetNewUserGift(ctx, credential)
	if err != nil {
		return nil, err
	}
	if r == nil {
		return nil, nil
	}
	gift, err := checkedGift(owner, *r)
	if err != nil {
		return nil, err
	}
	return &gift, nil
}

func (s *Gifts) ClaimStoredNewUserGift(ctx context.Context, credential string) (GiftClaim, error) {
	owner, err := s.owner(ctx, credential)
	if err != nil {
		return GiftClaim{}, err
	}
	r, already, err := s.port.ClaimStoredNewUserGift(ctx, credential)
	if err != nil {
		return GiftClaim{}, err
	}
	value, err := checkedGift(owner, r)
	if err != nil || value.Status != "claimed" {
		return GiftClaim{}, ErrGiftUnknown
	}
	return GiftClaim{Gift: value, AlreadyClaimed: already}, nil
}
