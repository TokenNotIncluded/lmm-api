package composition

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestAssistantMissingCredentialLeavesReservationAndModelUnavailable(t *testing.T) {
	r := &Runtime{}
	if err := r.composeAssistantDelegation(context.Background(), Config{}, nil, nil, nil); err != nil {
		t.Fatal(err)
	}
	if r.AssistantPending != nil || r.AssistantTurns != nil || r.AssistantModelClient != nil || r.AssistantChat != nil {
		t.Fatal("missing credential enabled an assistant port")
	}
	if token, err := loadAssistantCredential(Config{}, nil, nil, nil); err != nil || len(token) != 0 {
		t.Fatal("optional credential fell back to another identity")
	}
}

func TestAssistantCredentialRejectsReuseOfEveryLoadedRole(t *testing.T) {
	roles := [][]byte{[]byte(strings.Repeat("h", 32)), []byte(strings.Repeat("i", 32)), []byte(strings.Repeat("c", 32)), []byte(strings.Repeat("p", 32))}
	for i, name := range []string{"host", "identity", "commerce", "payment"} {
		t.Run(name, func(t *testing.T) {
			file := filepath.Join(t.TempDir(), "assistant-token")
			if os.WriteFile(file, roles[i], 0o600) != nil {
				t.Fatal("cannot write private test credential")
			}
			cfg := Config{Selected: []string{"assistant"}, HostCredential: roles[0], AssistantTokenFile: file}
			if _, err := loadAssistantCredential(cfg, roles[1], roles[2], roles[3]); err == nil {
				t.Fatal("assistant credential reused a loaded role")
			}
		})
	}
	if err := validateAssistantSelection(Config{AssistantTokenFile: "/tmp/private-assistant-token"}); err == nil {
		t.Fatal("unselected assistant token was ignored")
	}
}
