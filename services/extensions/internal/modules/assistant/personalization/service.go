package personalization

import (
	"bytes"
	"context"
	"database/sql"
	"encoding/json"
	"io"
	"strings"
	"time"
	"unicode/utf8"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
	services "github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant_services"
)

type Service struct {
	domain  *services.Service
	history *assistant.HistoryService
}
type greetingArguments struct {
	Language string `json:"language"`
	Template string `json:"template"`
}
type greetingDraft struct {
	Arguments        greetingArguments `json:"arguments"`
	ExpectedRevision int64             `json:"expected_revision"`
}

func decode(raw json.RawMessage, target any) error {
	d := json.NewDecoder(bytes.NewReader(raw))
	d.DisallowUnknownFields()
	if d.Decode(target) != nil {
		return access.ErrInvalid
	}
	var extra any
	if d.Decode(&extra) != io.EOF {
		return access.ErrInvalid
	}
	return nil
}

// New installs code-owned callbacks into the same durable service journal.
// Confirmation approval remains exclusively in Core's browser ceremony.
func New(domain *services.Service, history *assistant.HistoryService) (*Service, error) {
	if domain == nil || history == nil {
		return nil, access.ErrUnavailable
	}
	var err error
	domain, err = domain.WithToolAction("get_overview_greeting", 0, func(ctx context.Context, tx *sql.Tx, p access.Principal, in assistant.AgentInvocation) (any, error) {
		var args struct{}
		if decode(in.Call.Arguments, &args) != nil {
			return nil, access.ErrInvalid
		}
		return assistant.ReadGreetingTx(ctx, tx, p)
	})
	if err != nil {
		return nil, err
	}
	domain, err = domain.WithToolAction("set_conversation_title", 0, func(ctx context.Context, tx *sql.Tx, p access.Principal, in assistant.AgentInvocation) (any, error) {
		var args struct {
			Title string `json:"title"`
		}
		if decode(in.Call.Arguments, &args) != nil || !utf8.ValidString(args.Title) || len([]rune(args.Title)) < 2 || len([]rune(args.Title)) > 60 {
			return nil, access.ErrInvalid
		}
		var prior bool
		if err := tx.QueryRowContext(ctx, `SELECT EXISTS(SELECT 1 FROM lmm_assistant.turn_receipts r JOIN lmm_assistant.delegation_conversations c ON c.user_id=r.user_id AND c.conversation_id=r.conversation_id WHERE c.user_id=$1 AND c.core_conversation_id=$2)`, p.UserID, in.Call.CoreConversationID).Scan(&prior); err != nil {
			return nil, access.ErrUnavailable
		}
		if prior {
			return map[string]any{"applied": false, "new_conversation_only": true, "reason": "existing_conversation"}, nil
		}
		title := strings.TrimSpace(assistant.RedactAssistantHistoryContent(args.Title))
		if len([]rune(title)) < 2 || len([]rune(title)) > 60 {
			title = strings.TrimSpace(assistant.RedactAssistantHistoryContent(in.OriginalUserText))
			r := []rune(title)
			if len(r) > 60 {
				title = string(r[:60])
			}
		}
		return map[string]any{"title": title, "applied": false, "new_conversation_only": true}, nil
	})
	if err != nil {
		return nil, err
	}
	domain, err = domain.WithConfirmationAction("set_overview_greeting", 0, func(ctx context.Context, tx *sql.Tx, p access.Principal, raw json.RawMessage) (any, error) {
		var draft greetingDraft
		if decode(raw, &draft) != nil {
			return nil, access.ErrInvalid
		}
		return assistant.UpdateGreetingTx(ctx, tx, p, draft.Arguments.Language, draft.Arguments.Template, draft.ExpectedRevision, time.Now().Unix())
	})
	if err != nil {
		return nil, err
	}
	return &Service{domain: domain, history: history}, nil
}

func (s *Service) Domain() *services.Service { return s.domain }
func (s *Service) Bindings() []assistant.AgentToolBinding {
	out := make([]assistant.AgentToolBinding, 0, len(definitions))
	for _, d := range definitions {
		out = append(out, assistant.AgentToolBinding{Name: d.name, Description: d.description, Effect: d.effect, Access: d.access, Version: assistant.FixedMainToolVersion, PriceVersion: assistant.BuiltinToolPriceVersion, PriceSHA256: assistant.FreeAgentPriceSHA256(), Parameters: d.schema, Executor: s})
	}
	return out
}

func (s *Service) AvailableBindings() []assistant.AgentToolBinding {
	out := s.Bindings()
	if s.domain.ConfirmationAvailable() {
		return out
	}
	filtered := out[:0]
	for _, binding := range out {
		if binding.Name != "set_overview_greeting" {
			filtered = append(filtered, binding)
		}
	}
	return filtered
}
func (s *Service) Inspect(ctx context.Context, in assistant.AgentInvocation) (assistant.AgentToolResult, error) {
	return s.domain.Inspect(ctx, in)
}
func (s *Service) Execute(ctx context.Context, in assistant.AgentInvocation) (assistant.AgentToolResult, error) {
	if in.Call.Scope.Name != "set_overview_greeting" {
		return s.domain.Execute(ctx, in)
	}
	var args greetingArguments
	if decode(in.Call.Arguments, &args) != nil {
		return assistant.AgentToolResult{}, access.ErrInvalid
	}
	var fields map[string]json.RawMessage
	if json.Unmarshal(in.Call.Arguments, &fields) != nil || fields["language"] == nil || fields["template"] == nil || bytes.Equal(fields["language"], []byte("null")) || bytes.Equal(fields["template"], []byte("null")) || !assistant.ValidGreetingTemplate(args.Language, args.Template) {
		return assistant.AgentToolResult{}, access.ErrInvalid
	}
	// The read is itself journaled, while the exact revision is frozen into the
	// reviewed draft. Confirmation never substitutes the then-current revision.
	preference, err := s.history.Greeting(ctx, in.Credential)
	if err != nil {
		return assistant.AgentToolResult{}, err
	}
	raw, err := json.Marshal(greetingDraft{Arguments: args, ExpectedRevision: preference.Revision})
	if err != nil {
		return assistant.AgentToolResult{}, access.ErrInvalid
	}
	return s.domain.Prepare(ctx, in, raw)
}
