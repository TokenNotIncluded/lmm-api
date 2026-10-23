package assistant

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"strings"
	"unicode/utf8"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
)

// PersonalChat accepts no actor, payer, model or budget from the browser. Core
// binds those to the original Session and persists the billed result.
type PersonalChat struct {
	pending *PendingConversations
	turns   *DelegatedTurns
	history *HistoryService
	gifts   *Gifts
}

type PersonalChatRequest struct {
	ConversationID int64  `json:"conversation_id"`
	ClientTurnID   string `json:"client_turn_id"`
	Message        string `json:"message"`
}

type PersonalChatReply struct {
	ConversationID int64        `json:"conversation_id"`
	ClientTurnID   string       `json:"client_turn_id"`
	State          string       `json:"state"`
	Text           string       `json:"text,omitempty"`
	Gift           *NewUserGift `json:"gift,omitempty"`
}

func NewPersonalChat(pending *PendingConversations, turns *DelegatedTurns, history *HistoryService, gifts ...*Gifts) (*PersonalChat, error) {
	if pending == nil || turns == nil || history == nil || len(gifts) > 1 {
		return nil, ErrDelegationUnavailable
	}
	chat := &PersonalChat{pending: pending, turns: turns, history: history}
	if len(gifts) == 1 {
		chat.gifts = gifts[0]
	}
	return chat, nil
}

func (s *PersonalChat) GiftsAvailable() bool { return s != nil && s.gifts != nil }

func (s *PersonalChat) GetNewUserGift(ctx context.Context, credential string) (*NewUserGift, error) {
	if !s.GiftsAvailable() {
		return nil, ErrDelegationUnavailable
	}
	return s.gifts.GetNewUserGift(ctx, credential)
}

func (s *PersonalChat) ClaimStoredNewUserGift(ctx context.Context, credential string) (GiftClaim, error) {
	if !s.GiftsAvailable() {
		return GiftClaim{}, ErrDelegationUnavailable
	}
	return s.gifts.ClaimStoredNewUserGift(ctx, credential)
}

func (s *PersonalChat) Chat(ctx context.Context, credential string, request PersonalChatRequest) (PersonalChatReply, error) {
	if request.ConversationID < 0 || request.ConversationID > maxSafeHistoryID || !access.ID(request.ClientTurnID) || len(request.ClientTurnID) > 64 || !utf8.ValidString(request.Message) || len(request.Message) > 8192 || strings.TrimSpace(request.Message) == "" {
		return PersonalChatReply{}, access.ErrInvalid
	}
	id := request.ConversationID
	if id == 0 {
		// The same first-turn retry always inspects the same durable creation
		// claim. A lost ACK never invents a second conversation or payer.
		sum := sha256.Sum256([]byte("personal-chat-v1:" + request.ClientTurnID))
		pending, err := s.pending.Start(ctx, credential, hex.EncodeToString(sum[:16]))
		if err != nil {
			return PersonalChatReply{}, err
		}
		id = pending.ID
	}
	modelRequest := ModelRequest{Input: request.Message}
	if _, err := s.turns.OpenTurn(ctx, credential, id, request.ClientTurnID, modelRequest); err != nil {
		return PersonalChatReply{}, err
	}
	operation, err := s.turns.ReserveRound(ctx, credential, id, request.ClientTurnID, 0, modelRequest)
	if err != nil {
		return PersonalChatReply{}, err
	}
	return s.complete(ctx, credential, id, request.ClientTurnID, request.Message, operation)
}

// Recover reads only the original round and stored input. It cannot issue a
// grant, reserve funds, invoke a model or replace an unknown operation.
func (s *PersonalChat) Recover(ctx context.Context, credential string, conversationID int64, turnID string) (PersonalChatReply, error) {
	if conversationID <= 0 || conversationID > maxSafeHistoryID || !access.ID(turnID) || len(turnID) > 64 {
		return PersonalChatReply{}, access.ErrInvalid
	}
	operation, err := s.turns.RecoverRound(ctx, credential, conversationID, turnID, 0)
	if err != nil {
		return PersonalChatReply{}, err
	}
	owner, err := s.turns.owner(ctx, credential)
	if err != nil {
		return PersonalChatReply{}, err
	}
	var input string
	err = s.turns.journal.Within(ctx, owner, conversationID, turnID, func(state *DelegationLocalState) error {
		if state.Turn == nil || len(state.Turn.Rounds) == 0 {
			return access.ErrNotFound
		}
		input = state.Turn.Rounds[0].Reservation.Text
		return nil
	})
	if err != nil {
		return PersonalChatReply{}, err
	}
	return s.complete(ctx, credential, conversationID, turnID, input, operation)
}

func (s *PersonalChat) complete(ctx context.Context, credential string, conversationID int64, turnID, input string, operation PersonalTextOperation) (PersonalChatReply, error) {
	reply := PersonalChatReply{ConversationID: conversationID, ClientTurnID: turnID, State: operation.Status}
	if operation.Status != "settled" {
		return reply, nil
	}
	// Admission-only installations have no reply capability. Never fabricate
	// an answer or append history merely because a hold/settlement exists.
	if operation.AnswerText == nil {
		return reply, ErrDelegationUnavailable
	}
	if operation.GiftReceipt != nil {
		owner, err := s.turns.owner(ctx, credential)
		if err != nil {
			return reply, err
		}
		gift, err := checkedGift(owner, *operation.GiftReceipt)
		if err != nil {
			return reply, err
		}
		reply.Gift = &gift
	}
	id, err := s.history.RecordCompletedTurn(ctx, credential, conversationID, turnID, input, *operation.AnswerText)
	if err != nil {
		return reply, err
	}
	if id != conversationID {
		return reply, access.ErrConflict
	}
	reply.Text = RedactAssistantHistoryContent(*operation.AnswerText)
	return reply, nil
}
