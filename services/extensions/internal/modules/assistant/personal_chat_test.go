package assistant

import (
	"context"
	"errors"
	"strings"
	"testing"
)

type chatHistoryFixture struct{ historySpy }

func (r *chatHistoryFixture) RecordTurn(_ context.Context, owner int64, turn CompletedTurn) (int64, error) {
	r.calls++
	r.userID, r.turn = owner, turn
	return turn.ConversationID, nil
}

// Consumer ordering tests only; real Core/Go PostgreSQL and provider dispatch
// are independently verified by the combined process acceptance runner.
func TestPersonalChatPendingThenRecoveryRecordsOnlyAuthoritativeAnswer(t *testing.T) {
	turns, journal, port, authority := durableFixture(t)
	repo := &chatHistoryFixture{}
	history, err := NewHistory(repo, authority, []byte(strings.Repeat("k", 32)))
	if err != nil {
		t.Fatal(err)
	}
	chat, err := NewPersonalChat(&PendingConversations{}, turns, history)
	if err != nil {
		t.Fatal(err)
	}
	request := PersonalChatRequest{ConversationID: 10, ClientTurnID: "turn", Message: "question"}
	reply, err := chat.Chat(context.Background(), "session", request)
	if err != nil || reply.State != "reserved" || reply.Text != "" || repo.calls != 0 {
		t.Fatal(reply, err, repo.calls)
	}
	port.mu.Lock()
	op := port.operations["turn:0"]
	answer := "Answer with a@example.com"
	charged := int64(3)
	op.Status, op.SettledCredits, op.AnswerText = "settled", &charged, &answer
	gift := claimedGiftFixture()
	op.GiftReceipt = &gift
	port.operations["turn:0"] = op
	port.mu.Unlock()
	restartedTurns, err := NewDelegatedTurns(authority, journal, port)
	if err != nil {
		t.Fatal(err)
	}
	restarted, err := NewPersonalChat(&PendingConversations{}, restartedTurns, history)
	if err != nil {
		t.Fatal(err)
	}
	reply, err = restarted.Recover(context.Background(), "session", 10, "turn")
	if err != nil || reply.State != "settled" || reply.ConversationID != 10 || reply.Gift == nil || reply.Gift.Status != "claimed" || strings.Contains(reply.Text, "a@example.com") || repo.calls != 1 || port.reserveCalls != 1 {
		t.Fatal(reply, err, repo.calls, port.reserveCalls)
	}
	if repo.turn.UserContent != "question" || strings.Contains(repo.turn.AnswerContent, "a@example.com") {
		t.Fatal("untrusted or unredacted history")
	}
}

func TestPersonalChatRejectsGiftForAnotherSessionOwnerBeforeHistory(t *testing.T) {
	turns, _, port, authority := durableFixture(t)
	repo := &chatHistoryFixture{}
	history, _ := NewHistory(repo, authority, []byte(strings.Repeat("k", 32)))
	chat, _ := NewPersonalChat(&PendingConversations{}, turns, history)
	_, err := chat.Chat(context.Background(), "session", PersonalChatRequest{ConversationID: 10, ClientTurnID: "turn", Message: "question"})
	if err != nil {
		t.Fatal(err)
	}
	op := port.operations["turn:0"]
	charged, answer := int64(3), "answer"
	gift := claimedGiftFixture()
	gift.OwnerUserID = 2
	op.Status, op.SettledCredits, op.AnswerText, op.GiftReceipt = "settled", &charged, &answer, &gift
	port.operations["turn:0"] = op
	if _, err = chat.Recover(context.Background(), "session", 10, "turn"); !errors.Is(err, ErrGiftUnknown) || repo.calls != 0 {
		t.Fatal("cross-owner gift reached history", err, repo.calls)
	}
}

func TestPersonalChatAdmissionSettlementCannotInventAnswer(t *testing.T) {
	turns, _, port, authority := durableFixture(t)
	repo := &chatHistoryFixture{}
	history, _ := NewHistory(repo, authority, []byte(strings.Repeat("k", 32)))
	chat, _ := NewPersonalChat(&PendingConversations{}, turns, history)
	_, err := chat.Chat(context.Background(), "session", PersonalChatRequest{ConversationID: 10, ClientTurnID: "turn", Message: "question"})
	if err != nil {
		t.Fatal(err)
	}
	op := port.operations["turn:0"]
	charged := int64(3)
	op.Status, op.SettledCredits = "settled", &charged
	port.operations["turn:0"] = op
	_, err = chat.Recover(context.Background(), "session", 10, "turn")
	if !errors.Is(err, ErrDelegationUnavailable) || repo.calls != 0 {
		t.Fatal(err, repo.calls)
	}
}

func TestPersonalChatBindingChangeDoesNotReserveAnotherOperation(t *testing.T) {
	turns, _, port, authority := durableFixture(t)
	repo := &chatHistoryFixture{}
	history, _ := NewHistory(repo, authority, []byte(strings.Repeat("k", 32)))
	chat, _ := NewPersonalChat(&PendingConversations{}, turns, history)
	request := PersonalChatRequest{ConversationID: 10, ClientTurnID: "turn", Message: "question"}
	if _, err := chat.Chat(context.Background(), "session", request); err != nil {
		t.Fatal(err)
	}
	request.Message = "changed"
	if _, err := chat.Chat(context.Background(), "session", request); err == nil || port.reserveCalls != 1 {
		t.Fatal(err, port.reserveCalls)
	}
}

func TestPersonalTextAnswerIsBoundToImmutableSettledReceipt(t *testing.T) {
	binding := PersonalTextBinding{ID: "lmm_ad_" + strings.Repeat("a", 64), CoreConversationID: "lmm_cv_" + strings.Repeat("c", 64), TurnID: "turn", OperationKey: "turn:0", TextSHA256: strings.Repeat("d", 64)}
	charged, answer := int64(3), "answer"
	op := PersonalTextOperation{Binding: binding, RequestID: strings.Repeat("b", 64), Status: "settled", ReservedCredits: 10, SettledCredits: &charged, AnswerText: &answer}
	if _, err := checkedPersonalTextOperation(binding, op, nil); err != nil {
		t.Fatal(err)
	}
	for _, invalid := range []string{"", strings.Repeat("a", 32769), string([]byte{0xff})} {
		bad := op
		bad.AnswerText = &invalid
		if _, err := checkedPersonalTextOperation(binding, bad, nil); err == nil {
			t.Fatal("accepted malformed answer")
		}
	}
	bad := op
	bad.Status, bad.SettledCredits = "reserved", nil
	if _, err := checkedPersonalTextOperation(binding, bad, nil); err == nil {
		t.Fatal("reserved receipt carried a provider answer")
	}
	changed := "different"
	bad = op
	bad.AnswerText = &changed
	if validPersonalTextTransition(op, bad) {
		t.Fatal("settled answer was mutable")
	}
}
