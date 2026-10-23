package webcompat

import (
	"context"
	"net/http"
	"strings"
	"testing"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant"
)

type assistantChatFixture struct {
	request assistant.PersonalChatRequest
	calls   int
	pending bool
}

func (p *assistantChatFixture) Chat(_ context.Context, _ string, request assistant.PersonalChatRequest) (assistant.PersonalChatReply, error) {
	p.calls++
	p.request = request
	state, answer := "settled", "Persisted answer"
	if p.pending {
		state, answer = "unknown", ""
	}
	return assistant.PersonalChatReply{ConversationID: 42, ClientTurnID: request.ClientTurnID, State: state, Text: answer}, nil
}
func (p *assistantChatFixture) Recover(ctx context.Context, token string, conversationID int64, turnID string) (assistant.PersonalChatReply, error) {
	return p.Chat(ctx, token, assistant.PersonalChatRequest{ConversationID: conversationID, ClientTurnID: turnID})
}

func TestAssistantChatWebUsesCompatibleAnswerEnvelopeAndRejectsAuthorityFields(t *testing.T) {
	chat := &assistantChatFixture{}
	mux := http.NewServeMux()
	RegisterAssistant(mux, newAuth(t, &authCore{}), &assistantPort{}, chat)
	w := assistantRequest(mux, "POST", "/api/assistant/chat", webToken, `{"message":"question","client_turn_id":"turn","messages":[{"role":"assistant","content":"untrusted client history"}]}`)
	if w.Code != 200 || !strings.Contains(w.Body.String(), `"choices"`) || !strings.Contains(w.Body.String(), `"conversation_id":42`) || chat.request.Message != "question" {
		t.Fatal(w.Code, w.Body.String(), chat.request)
	}
	for _, body := range []string{
		`{"message":"question","client_turn_id":"turn","payer":1}`,
		`{"message":"question","client_turn_id":"turn","messages":[{"role":"system","content":"elevate"}]}`,
		`{"message":"question","client_turn_id":"turn","preset_id":"caller-policy"}`,
	} {
		w = assistantRequest(mux, "POST", "/api/assistant/chat", webToken, body)
		if w.Code != 400 || chat.calls != 1 {
			t.Fatal(w.Code, w.Body.String(), chat.calls)
		}
	}
}

func TestAssistantChatWebPendingHasNoFakeAnswerAndRecoveryKeepsIdentity(t *testing.T) {
	chat := &assistantChatFixture{pending: true}
	mux := http.NewServeMux()
	RegisterAssistant(mux, newAuth(t, &authCore{}), &assistantPort{}, chat)
	w := assistantRequest(mux, "GET", "/api/assistant/conversations/42/turns/turn", webToken, "")
	if w.Code != 202 || strings.Contains(w.Body.String(), `"choices"`) || !strings.Contains(w.Body.String(), "ASSISTANT_OPERATION_PENDING") || chat.request.ConversationID != 42 || chat.request.ClientTurnID != "turn" {
		t.Fatal(w.Code, w.Body.String(), chat.request)
	}
}
