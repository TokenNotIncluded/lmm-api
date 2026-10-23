package assistant

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"sort"
	"strings"
	"unicode/utf8"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/redaction"
)

const AgentTurnFeature = "assistant.agent_turn.execute.v1"
const FixedMainToolVersion = "main-3948a0a1"
const BuiltinToolPriceVersion = "builtin-free-v1"
const maxAgentJSON = 64 << 10

// Bindings are trusted installed code, not registrations from a provider, a
// browser or an extension result. Core independently installs the same scope.
type AgentToolBinding struct {
	Name         string
	Description  string
	Effect       string
	Access       string
	Version      string
	PriceVersion string
	PriceSHA256  string
	Parameters   json.RawMessage
	Executor     AgentToolExecutor
}
type AgentRegistry struct{ bindings map[string]AgentToolBinding }

type AgentToolDescriptor struct {
	Name         string          `json:"name"`
	Description  string          `json:"description"`
	Version      string          `json:"version"`
	PriceVersion string          `json:"price_version"`
	PriceSHA256  string          `json:"price_sha256"`
	SchemaSHA256 string          `json:"schema_sha256"`
	Effect       string          `json:"effect"`
	Access       string          `json:"access"`
	Domain       string          `json:"domain"`
	Parameters   json.RawMessage `json:"parameters"`
}

// Descriptors is an operator-review/install artifact. Sending this list over a
// model request never registers it or extends the Core grant's tool scope.
func (r *AgentRegistry) Descriptors() []AgentToolDescriptor {
	if r == nil {
		return nil
	}
	names := make([]string, 0, len(r.bindings))
	for name := range r.bindings {
		names = append(names, name)
	}
	sort.Strings(names)
	out := make([]AgentToolDescriptor, 0, len(names))
	for _, name := range names {
		b := r.bindings[name]
		out = append(out, AgentToolDescriptor{Name: b.Name, Description: b.Description, Version: b.Version, PriceVersion: b.PriceVersion, PriceSHA256: b.PriceSHA256, SchemaSHA256: agentDigest(b.Parameters), Effect: b.Effect, Access: b.Access, Domain: "extension", Parameters: append(json.RawMessage(nil), b.Parameters...)})
	}
	return out
}

func agentDigest(data []byte) string { h := sha256.Sum256(data); return hex.EncodeToString(h[:]) }

func FreeAgentPriceSHA256() string {
	return agentDigest([]byte(`{"kind":"free","unit":"credit_500k_usd","version":"builtin-free-v1"}`))
}

func agentJSONObject(data []byte) bool {
	if len(data) == 0 || len(data) > maxAgentJSON || !utf8.Valid(data) || !strings.HasPrefix(strings.TrimSpace(string(data)), "{") {
		return false
	}
	_, err := redaction.JSON(data)
	return err == nil
}

func validAgentKey(key AgentTurnKey) bool {
	return coreConversationID(key.CoreConversationID) && access.ID(key.ClientTurnID) && len(key.ClientTurnID) <= 64
}

func validAgentCall(call AgentToolCall) bool {
	if !validAgentKey(AgentTurnKey{call.CoreConversationID, call.TurnID}) || !access.ID(call.OperationID) || !access.ID(call.ModelOperationID) || !access.ID(call.ProviderCallID) || call.Round >= 32 || call.Ordinal >= 8 || call.Scope.Domain != "extension" || !delegationLabel(call.Scope.Name, 80) || !delegationLabel(call.Scope.Version, 128) || !delegationLabel(call.Scope.PriceVersion, 128) || !lowerDigest(call.Scope.PriceSHA256) || !lowerDigest(call.Scope.SchemaSHA256) || !agentJSONObject(call.Arguments) || agentDigest(call.Arguments) != call.ArgumentsSHA256 {
		return false
	}
	for _, v := range []string{call.Scope.ResourceKind, call.Scope.ResourceID, call.Scope.ResourceVersion, call.Scope.GrantID} {
		if v != "" && !delegationLabel(v, 128) {
			return false
		}
	}
	if call.Scope.ResourceSchemaSHA256 != "" && !lowerDigest(call.Scope.ResourceSchemaSHA256) {
		return false
	}
	if call.Scope.ResourcePriceSHA256 != "" && !lowerDigest(call.Scope.ResourcePriceSHA256) {
		return false
	}
	if call.Scope.Name == "call_market_tool" && (call.Scope.ResourceKind != "toolmarket" || call.Scope.ResourceID == "" || call.Scope.ResourceVersion == "" || call.Scope.GrantID == "" || !lowerDigest(call.Scope.ResourceSchemaSHA256) || !lowerDigest(call.Scope.ResourcePriceSHA256)) {
		return false
	}
	return true
}

func NewAgentRegistry(bindings []AgentToolBinding) (*AgentRegistry, error) {
	if len(bindings) == 0 || len(bindings) > 128 {
		return nil, access.ErrInvalid
	}
	r := &AgentRegistry{bindings: make(map[string]AgentToolBinding, len(bindings))}
	for _, b := range bindings {
		if !delegationLabel(b.Name, 80) || !utf8.ValidString(b.Description) || len(b.Description) == 0 || len(b.Description) > 4096 || !delegationLabel(b.Version, 128) || !delegationLabel(b.PriceVersion, 128) || !lowerDigest(b.PriceSHA256) || !agentJSONObject(b.Parameters) || access.Missing(b.Executor) {
			return nil, access.ErrInvalid
		}
		switch b.Effect {
		case "read_only", "server_guarded", "navigation", "confirmation":
		default:
			return nil, access.ErrInvalid
		}
		switch b.Access {
		case "user", "l0", "l1", "admin", "root", "mixed":
		default:
			return nil, access.ErrInvalid
		}
		if _, found := r.bindings[b.Name]; found {
			return nil, access.ErrConflict
		}
		b.Parameters = append(json.RawMessage(nil), b.Parameters...)
		r.bindings[b.Name] = b
	}
	return r, nil
}

func (r *AgentRegistry) Scope(name string) (AgentToolScope, error) {
	b, ok := r.bindings[name]
	if !ok {
		return AgentToolScope{}, access.ErrNotFound
	}
	return AgentToolScope{Name: b.Name, Version: b.Version, PriceVersion: b.PriceVersion, PriceSHA256: b.PriceSHA256, Domain: "extension", SchemaSHA256: agentDigest(b.Parameters)}, nil
}

func (r *AgentRegistry) dispatch(ctx context.Context, in AgentInvocation, inspect bool) (result AgentToolResult, err error) {
	if r == nil || !validAgentCall(in.Call) || in.Actor.UserID <= 0 || !in.Actor.Account.Valid() || in.Credential == "" {
		return AgentToolResult{}, access.ErrInvalid
	}
	b, ok := r.bindings[in.Call.Scope.Name]
	if !ok {
		return AgentToolResult{}, access.ErrNotFound
	}
	if b.Version != in.Call.Scope.Version || b.PriceVersion != in.Call.Scope.PriceVersion || b.PriceSHA256 != in.Call.Scope.PriceSHA256 || agentDigest(b.Parameters) != in.Call.Scope.SchemaSHA256 {
		return AgentToolResult{}, access.ErrConflict
	}
	defer func() {
		if recover() != nil {
			result = AgentToolResult{}
			err = ErrDelegationUnknown
		}
	}()
	if inspect {
		result, err = b.Executor.Inspect(ctx, in)
	} else {
		result, err = b.Executor.Execute(ctx, in)
	}
	if err != nil {
		return AgentToolResult{}, safeToolError(err)
	}
	return checkedAgentResult(in.Call, result, in.Credential)
}
func (r *AgentRegistry) Execute(ctx context.Context, in AgentInvocation) (AgentToolResult, error) {
	return r.dispatch(ctx, in, false)
}
func (r *AgentRegistry) Inspect(ctx context.Context, in AgentInvocation) (AgentToolResult, error) {
	return r.dispatch(ctx, in, true)
}

func checkedAgentResult(call AgentToolCall, result AgentToolResult, credential string) (AgentToolResult, error) {
	if result.OperationID != call.OperationID || !agentJSONObject(result.ResultJSON) || result.ResultSHA256 != agentDigest(result.ResultJSON) {
		return AgentToolResult{}, ErrDelegationUnknown
	}
	switch result.State {
	case AgentToolSucceeded, AgentToolFailed:
		if !delegationLabel(result.DomainReceiptID, 256) {
			return AgentToolResult{}, ErrDelegationUnknown
		}
	case AgentToolPending, AgentToolUnknown:
	default:
		return AgentToolResult{}, ErrDelegationUnknown
	}
	if result.FinalText != "" && (call.Scope.Name != "end_conversation" || result.State != AgentToolSucceeded || !utf8.ValidString(result.FinalText) || len([]rune(result.FinalText)) > 2000) {
		return AgentToolResult{}, ErrDelegationUnknown
	}
	clean, err := redaction.JSON(result.ResultJSON, credential)
	if err != nil || len(clean) > maxAgentJSON {
		return AgentToolResult{}, ErrDelegationUnknown
	}
	result.ResultJSON = clean
	result.ResultSHA256 = agentDigest(clean)
	if credential != "" {
		result.FinalText = strings.ReplaceAll(result.FinalText, credential, "[REDACTED]")
	}
	result.FinalText = redaction.Text(result.FinalText)
	return result, nil
}

// AgentResult builds a bounded data result for trusted domain adapters. It does
// not assert that any business/financial action occurred; the domain receipt
// and independent Core checks still establish those facts.
func AgentResult(call AgentToolCall, state AgentToolState, value any, domainReceiptID string) (AgentToolResult, error) {
	data, err := json.Marshal(value)
	if err != nil || !agentJSONObject(data) {
		return AgentToolResult{}, access.ErrInvalid
	}
	result := AgentToolResult{OperationID: call.OperationID, State: state, ResultJSON: data, ResultSHA256: agentDigest(data), DomainReceiptID: domainReceiptID}
	return result, nil
}

func agentRecoveryError(err error) error {
	if errors.Is(err, access.ErrNotFound) {
		return ErrDelegationUnknown
	}
	return safeToolError(err)
}
