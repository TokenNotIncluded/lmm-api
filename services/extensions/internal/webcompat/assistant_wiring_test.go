package webcompat

import (
	"net/http"
	"strings"
	"testing"
)

func TestMountAssistantChatUsesRuntimePortAndSharedAdmission(t *testing.T) {
	host, core := mountTransportHost(t, nil)
	chat := &assistantChatFixture{}
	handler := Mount(host, newAuth(t, core), nil, nil, AdditionalServices{Assistant: &assistantPort{}, AssistantChat: chat})
	body := `{"message":"question","client_turn_id":"turn"}`
	if w := assistantRequest(handler, "POST", "/api/assistant/chat", webToken, body); w.Code != http.StatusServiceUnavailable || chat.calls != 0 {
		t.Fatal("preparing host dispatched chat", w.Code, chat.calls)
	}
	if err := host.MarkReady(); err != nil {
		t.Fatal(err)
	}
	w := assistantRequest(handler, "POST", "/api/assistant/chat", webToken, body)
	if w.Code != http.StatusOK || chat.calls != 1 || !strings.Contains(w.Body.String(), "Persisted answer") {
		t.Fatal("runtime chat was not mounted", w.Code, w.Body.String(), chat.calls)
	}
	w = assistantRequest(handler, "GET", "/api/assistant/status", webToken, "")
	if w.Code != http.StatusOK || !strings.Contains(w.Body.String(), `"route_available":true`) || !strings.Contains(w.Body.String(), `"mode":"personal"`) {
		t.Fatal(w.Code, w.Body.String())
	}
	host.BeginDrain()
	w = assistantRequest(handler, "POST", "/api/assistant/chat", webToken, body)
	if w.Code != http.StatusServiceUnavailable || chat.calls != 1 {
		t.Fatal("drained host dispatched chat", w.Code, chat.calls)
	}
}

func TestMountAssistantWithoutExecutionPortKeepsChatUnavailable(t *testing.T) {
	host, core := mountTransportHost(t, nil)
	handler := Mount(host, newAuth(t, core), nil, nil, AdditionalServices{Assistant: &assistantPort{}})
	if err := host.MarkReady(); err != nil {
		t.Fatal(err)
	}
	w := assistantRequest(handler, "POST", "/api/assistant/chat", webToken, `{"message":"question","client_turn_id":"turn"}`)
	if w.Code != http.StatusServiceUnavailable || strings.Contains(w.Body.String(), `"choices"`) {
		t.Fatal(w.Code, w.Body.String())
	}
}
