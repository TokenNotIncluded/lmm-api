package assistant_services

import (
	"context"
	"database/sql"
	"encoding/json"
	"github.com/TokenNotIncluded/lmm-api/services/extensions/internal/modules/assistant"
	"github.com/TokenNotIncluded/lmm-api/services/extensions/internal/modules/assistant/access"
	"strings"
	"unicode/utf8"
)

func (s *Service) dispatch(ctx context.Context, tx *sql.Tx, in assistant.AgentInvocation, p access.Principal, session [32]byte) (assistant.AgentToolResult, error) {
	if value, handled, err := s.memoryTool(ctx, tx, in, p); handled {
		if err != nil {
			return assistant.AgentToolResult{}, err
		}
		state := assistant.AgentToolSucceeded
		if object, ok := value.(map[string]any); ok && object["ok"] == false {
			state = assistant.AgentToolFailed
		}
		return result(in.Call.OperationID, state, value, "service:"+in.Call.OperationID)
	}
	if out, handled, err := s.issueTool(ctx, tx, in, p, session); handled {
		return out, err
	}
	if out, handled, err := s.supportTool(ctx, tx, in, p, session); handled {
		return out, err
	}
	for _, definition := range PureDefinitions() {
		if definition.Name == in.Call.Scope.Name {
			raw, err := ExecutePure(definition.Name, in.Call.Arguments)
			if err != nil {
				return assistant.AgentToolResult{}, access.ErrInvalid
			}
			var value map[string]any
			if json.Unmarshal(raw, &value) != nil {
				return assistant.AgentToolResult{}, access.ErrUnavailable
			}
			state := assistant.AgentToolSucceeded
			if value["ok"] == false {
				state = assistant.AgentToolFailed
			}
			return result(in.Call.OperationID, state, value, "service:"+in.Call.OperationID)
		}
	}
	switch in.Call.Scope.Name {
	case "get_service_facts":
		var input struct{}
		if decode(in.Call.Arguments, &input) != nil {
			return assistant.AgentToolResult{}, access.ErrInvalid
		}
		if access.Missing(s.dependencies.Public) {
			return assistant.AgentToolResult{}, access.ErrUnavailable
		}
		raw, err := s.dependencies.Public.ServiceFacts(ctx, in.Credential)
		if err != nil {
			return assistant.AgentToolResult{}, err
		}
		var value map[string]any
		if decode(raw, &value) != nil {
			return assistant.AgentToolResult{}, access.ErrUnavailable
		}
		return result(in.Call.OperationID, assistant.AgentToolSucceeded, value, "service:"+in.Call.OperationID)
	case "get_setup_guide":
		var input struct {
			Topic    string `json:"topic"`
			Platform string `json:"platform"`
			ModelID  string `json:"model_id"`
		}
		if decode(in.Call.Arguments, &input) != nil {
			return assistant.AgentToolResult{}, access.ErrInvalid
		}
		if access.Missing(s.dependencies.Public) {
			return assistant.AgentToolResult{}, access.ErrUnavailable
		}
		raw, err := s.dependencies.Public.SetupGuide(ctx, in.Credential, in.Call.Arguments)
		if err != nil {
			return assistant.AgentToolResult{}, err
		}
		var value map[string]any
		if decode(raw, &value) != nil {
			return assistant.AgentToolResult{}, access.ErrUnavailable
		}
		return result(in.Call.OperationID, assistant.AgentToolSucceeded, value, "service:"+in.Call.OperationID)
	case "navigate_to_page":
		return s.navigate(ctx, in, p)
	case "get_site_policy", "search_site_policies", "prepare_admin_site_policy_change":
		return s.policyTool(ctx, in, p)
	}
	if action, ok := s.actions[in.Call.Scope.Name]; ok {
		if p.Level < action.minimumLevel {
			return assistant.AgentToolResult{}, access.ErrForbidden
		}
		value, err := action.apply(ctx, tx, p, in)
		if err != nil {
			return assistant.AgentToolResult{}, err
		}
		return result(in.Call.OperationID, assistant.AgentToolSucceeded, value, "service:"+in.Call.OperationID)
	}
	return assistant.AgentToolResult{}, access.ErrNotFound
}

type NavigationInput struct {
	Page       string `json:"page"`
	Identifier string `json:"identifier"`
	Query      string `json:"query"`
	Section    string `json:"section"`
}
type NavigationReader interface {
	Navigation(context.Context, string, NavigationInput) (json.RawMessage, error)
}

func (s *Service) navigate(ctx context.Context, in assistant.AgentInvocation, p access.Principal) (assistant.AgentToolResult, error) {
	var input NavigationInput
	if decode(in.Call.Arguments, &input) != nil || utf8.RuneCountInString(input.Identifier) > 200 || utf8.RuneCountInString(input.Query) > 200 {
		return assistant.AgentToolResult{}, access.ErrInvalid
	}
	input.Page = strings.TrimSpace(input.Page)
	input.Identifier = strings.TrimSpace(input.Identifier)
	input.Query = strings.TrimSpace(input.Query)
	// Resource visibility, non-self target resolution, and developer access need
	// a typed live projection. No model-supplied ID proves any of those rights.
	if input.Page == "drawing" || input.Page == "store-product" || input.Page == "tool-market-service" || ((input.Page == "users" || input.Page == "usage-logs") && (input.Identifier != "" || input.Query != "")) {
		if access.Missing(s.dependencies.Navigation) {
			return assistant.AgentToolResult{}, access.ErrUnavailable
		}
		raw, err := s.dependencies.Navigation.Navigation(ctx, in.Credential, input)
		if err != nil {
			return assistant.AgentToolResult{}, err
		}
		var value map[string]any
		if decode(raw, &value) != nil {
			return assistant.AgentToolResult{}, access.ErrUnavailable
		}
		return result(in.Call.OperationID, assistant.AgentToolSucceeded, value, "service:"+in.Call.OperationID)
	}
	paths := map[string]string{"home": "/", "pricing": "/pricing", "wallet": "/wallet", "keys": "/keys", "models": "/models", "profile": "/profile", "support": "/support", "open-source-bounties": "/open-source-bounties", "store": "/store", "tool-market": "/tool-market", "users": "/users"}
	path, ok := paths[input.Page]
	query := map[string]any{}
	if input.Page == "models" && p.Level < 5 {
		path = "/pricing"
	}
	if input.Page == "users" {
		if p.Level < 5 {
			return assistant.AgentToolResult{}, access.ErrForbidden
		}
		query["l0Only"] = false
	}
	if input.Page == "usage-logs" {
		path = "/usage-logs/common"
		ok = true
		if input.Section == "common" || input.Section == "drawing" || input.Section == "task" {
			path = "/usage-logs/" + input.Section
		} else if input.Section != "" {
			return assistant.AgentToolResult{}, access.ErrInvalid
		}
	}
	if !ok {
		return assistant.AgentToolResult{}, access.ErrInvalid
	}
	return result(in.Call.OperationID, assistant.AgentToolSucceeded, map[string]any{"ok": true, "status": "navigation_ready", "path": path, "query": query}, "service:"+in.Call.OperationID)
}
