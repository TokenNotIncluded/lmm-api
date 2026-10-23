package webcompat

import (
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant"
)

func TestAssistantAgentTerminalAndHumanStates(t *testing.T) {
	for _, test := range []struct {
		state  string
		status int
		code   string
	}{{"rejected", 409, "ASSISTANT_OPERATION_REJECTED"}, {"expired", 409, "ASSISTANT_OPERATION_EXPIRED"}, {"human_support_pending", 202, "ASSISTANT_HUMAN_SUPPORT_PENDING"}} {
		w := httptest.NewRecorder()
		assistantChatRespond(w, assistant.PersonalChatReply{ConversationID: 1, ClientTurnID: "original", State: test.state}, nil)
		if w.Code != test.status || !strings.Contains(w.Body.String(), test.code) || strings.Contains(w.Body.String(), `"choices"`) {
			t.Fatalf("%s: %d %s", test.state, w.Code, w.Body.String())
		}
	}
}
