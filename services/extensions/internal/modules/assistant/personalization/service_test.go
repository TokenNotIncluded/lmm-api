package personalization

import (
	"context"
	"encoding/json"
	"errors"
	"strings"
	"testing"

	"github.com/TokenNotIncluded/lmm-api/services/extensions/internal/modules/assistant"
	"github.com/TokenNotIncluded/lmm-api/services/extensions/internal/modules/assistant/access"
)

func TestGreetingPreviewRejectsMalformedArgumentsBeforeDomainAccess(t *testing.T) {
	for _, raw := range []string{`{}`, `{"language":"en"}`, `{"language":"en","template":null}`, `{"language":"xx","template":"ok"}`, `{"language":"en","template":"$unknown"}`, `{"language":"en","template":"ok","confirm":true}`, `{"language":"en","template":"` + strings.Repeat("a", 513) + `"}`} {
		in := assistant.AgentInvocation{Call: assistant.AgentToolCall{Scope: assistant.AgentToolScope{Name: "set_overview_greeting"}, Arguments: json.RawMessage(raw)}}
		_, err := (&Service{}).Execute(context.Background(), in)
		if !errors.Is(err, access.ErrInvalid) {
			t.Fatalf("%s: %v", raw, err)
		}
	}
}

func TestInstalledPersonalizationDefinitions(t *testing.T) {
	bindings := (&Service{}).Bindings()
	if len(bindings) != 3 {
		t.Fatal(len(bindings))
	}
	r, err := assistant.NewAgentRegistry(bindings)
	if err != nil {
		t.Fatal(err)
	}
	want := map[string]bool{"get_overview_greeting": true, "set_overview_greeting": true, "set_conversation_title": true}
	for _, d := range r.Descriptors() {
		if _, ok := want[d.Name]; !ok || d.Version != assistant.FixedMainToolVersion || d.PriceSHA256 != assistant.FreeAgentPriceSHA256() {
			t.Fatalf("unexpected definition %+v", d)
		}
		delete(want, d.Name)
	}
	if len(want) != 0 {
		t.Fatal(want)
	}
}
