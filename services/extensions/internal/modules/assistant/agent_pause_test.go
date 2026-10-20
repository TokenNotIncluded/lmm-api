package assistant

import (
	"context"
	"testing"
)

type pausedHumanSupport struct{}

func (pausedHumanSupport) HumanSupportActive(context.Context, string, string) (bool, error) {
	return true, nil
}
func TestAgentChatHumanSupportPausesBeforeModelIntent(t *testing.T) {
	chat, journal, port, executor, history := agentChatFixture(t)
	chat = chat.WithHumanSupportGuard(pausedHumanSupport{})
	reply, err := chat.Chat(context.Background(), "session", agentRequest)
	if err != nil || reply.State != "human_support_pending" {
		t.Fatalf("%+v %v", reply, err)
	}
	if journal.state.Turn != nil || port.starts != 0 || port.claims != 0 || executor.executes != 0 || history.calls != 0 {
		t.Fatal("human handoff started AI work")
	}
}
