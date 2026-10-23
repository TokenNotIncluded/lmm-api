package assistant

import (
	"crypto/sha256"
	"encoding/hex"
	"strconv"
	"unicode/utf8"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
)

// Personal text admission is a separate Core capability. Its text digest is
// NOT the legacy typed ModelRequest JSON digest; its receipt has no provider
// answer, tools, worker credential or authority to run an upstream in Go.
type PersonalTextGrant struct {
	ID                 string `json:"delegation_id"`
	CoreConversationID string `json:"core_conversation_id"`
	TurnID             string `json:"turn_id"`
	ExpiresAt          int64  `json:"expires_at_unix"`
	MaxModelCalls      int    `json:"max_model_calls"`
	Revoked            bool   `json:"revoked"`
}

type PersonalTextTurnKey struct {
	CoreConversationID string
	TurnID             string
}

type PersonalTextBinding struct {
	ID                 string `json:"delegation_id"`
	CoreConversationID string `json:"core_conversation_id"`
	TurnID             string `json:"turn_id"`
	Round              int    `json:"round"`
	OperationKey       string `json:"operation_key"`
	TextSHA256         string `json:"text_sha256"`
}

type PersonalTextReservation struct {
	Binding PersonalTextBinding `json:"binding"`
	Text    string              `json:"text"`
}

type PersonalTextOperation struct {
	Binding         PersonalTextBinding `json:"binding"`
	RequestID       string              `json:"request_id"`
	Status          string              `json:"status"`
	ReservedCredits int64               `json:"reserved_credits"`
	SettledCredits  *int64              `json:"settled_credits"`
	AnswerText      *string             `json:"answer_text,omitempty"`
	GiftReceipt     *GiftReceipt        `json:"gift_receipt,omitempty"`
}

func validPersonalTextGrant(g PersonalTextGrant) bool {
	return delegationID(g.ID) && coreConversationID(g.CoreConversationID) && delegationLabel(g.TurnID, 124) && g.ExpiresAt > 0 && g.MaxModelCalls >= 1 && g.MaxModelCalls <= 4
}

func validPersonalTextBinding(b PersonalTextBinding) bool {
	return delegationID(b.ID) && coreConversationID(b.CoreConversationID) && delegationLabel(b.TurnID, 124) && b.Round >= 0 && b.Round < 4 && b.OperationKey == b.TurnID+":"+strconv.Itoa(b.Round) && len(b.OperationKey) <= 128 && lowerDigest(b.TextSHA256)
}

func validPersonalTextReservation(r PersonalTextReservation) bool {
	if !validPersonalTextBinding(r.Binding) || !utf8.ValidString(r.Text) || len(r.Text) == 0 || len(r.Text) > 8192 {
		return false
	}
	sum := sha256.Sum256([]byte(r.Text))
	return hex.EncodeToString(sum[:]) == r.Binding.TextSHA256
}

func checkedPersonalTextOperation(expected PersonalTextBinding, receipt PersonalTextOperation, err error) (PersonalTextOperation, error) {
	if err != nil {
		return PersonalTextOperation{}, recoveryError(err)
	}
	if !validPersonalTextBinding(expected) || receipt.Binding != expected || !lowerDigest(receipt.RequestID) || receipt.ReservedCredits <= 0 || receipt.ReservedCredits > maxSafeHistoryID {
		return PersonalTextOperation{}, access.ErrUnavailable
	}
	switch receipt.Status {
	case "reserved", "streaming", "released", "unknown":
		if receipt.SettledCredits != nil || receipt.AnswerText != nil || receipt.GiftReceipt != nil {
			return PersonalTextOperation{}, access.ErrUnavailable
		}
	case "settled":
		if receipt.SettledCredits == nil || *receipt.SettledCredits < 0 || *receipt.SettledCredits > receipt.ReservedCredits {
			return PersonalTextOperation{}, access.ErrUnavailable
		}
		if receipt.AnswerText != nil && (!utf8.ValidString(*receipt.AnswerText) || len(*receipt.AnswerText) == 0 || len(*receipt.AnswerText) > 32768) {
			return PersonalTextOperation{}, access.ErrUnavailable
		}
		if receipt.GiftReceipt != nil {
			if _, err := checkedGift(receipt.GiftReceipt.OwnerUserID, *receipt.GiftReceipt); err != nil {
				return PersonalTextOperation{}, access.ErrUnavailable
			}
		}
	default:
		return PersonalTextOperation{}, access.ErrUnavailable
	}
	return receipt, nil
}
