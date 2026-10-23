package webcompat

import (
	"context"
	"net/http"
	"testing"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant"
)

type assistantResumeFixture struct {
	assistantChatFixture
	resumes      int
	conversation int64
	turn         string
}

func (p *assistantResumeFixture) Resume(_ context.Context, _ string, id int64, turn string) (assistant.PersonalChatReply, error) {
	p.resumes++
	p.conversation, p.turn = id, turn
	return assistant.PersonalChatReply{ConversationID: id, ClientTurnID: turn, State: "settled", Text: "Original answer"}, nil
}
func TestAssistantResumeRequiresEmptyCommandAndKeepsOriginalIdentity(t *testing.T) {
	chat := &assistantResumeFixture{}
	mux := http.NewServeMux()
	RegisterAssistant(mux, newAuth(t, &authCore{}), &assistantPort{}, chat)
	path := "/api/assistant/conversations/42/turns/original/resume"
	w := assistantRequest(mux, "POST", path, webToken, `{}`)
	if w.Code != 200 || chat.resumes != 1 || chat.conversation != 42 || chat.turn != "original" {
		t.Fatal(w.Code, w.Body.String(), chat)
	}
	for _, body := range []string{`{"message":"replacement"}`, `{"amount":1}`, `{"owner":2}`, `{"payer":"platform"}`, `{"tools":[]}`} {
		w = assistantRequest(mux, "POST", path, webToken, body)
		if w.Code != 400 || chat.resumes != 1 {
			t.Fatal(w.Code, w.Body.String(), body)
		}
	}
	w = assistantRequest(mux, "POST", path+"?amount=1", webToken, `{}`)
	if w.Code != 400 || chat.resumes != 1 {
		t.Fatal(w.Code, w.Body.String())
	}
	w = assistantRequest(mux, "POST", path, "", `{}`)
	if w.Code != 401 || chat.resumes != 1 {
		t.Fatal(w.Code, w.Body.String())
	}
}
