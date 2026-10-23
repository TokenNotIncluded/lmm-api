package assistant

import (
	"context"
	"encoding/json"

	"github.com/TokenNotIncluded/api.lmm.best/extensions/internal/modules/assistant/access"
)

// AgentToolScope is copied only from an authenticated Core round receipt. A
// definition or tool result cannot create this authority. Version/digests freeze
// the installed tool and, where relevant, its independently approved resource.
type AgentToolScope struct {
	Name                 string `json:"name"`
	Version              string `json:"version"`
	SchemaSHA256         string `json:"schema_sha256"`
	PriceVersion         string `json:"price_version"`
	PriceSHA256          string `json:"price_sha256"`
	Domain               string `json:"domain"`
	ResourceKind         string `json:"resource_kind,omitempty"`
	ResourceID           string `json:"resource_id,omitempty"`
	ResourceVersion      string `json:"resource_version,omitempty"`
	ResourceSchemaSHA256 string `json:"resource_schema_sha256,omitempty"`
	ResourcePriceSHA256  string `json:"resource_price_sha256,omitempty"`
	GrantID              string `json:"grant_id,omitempty"`
}

type AgentToolCall struct {
	OperationID        string          `json:"operation_id"`
	CoreConversationID string          `json:"core_conversation_id"`
	TurnID             string          `json:"turn_id"`
	Round              uint32          `json:"round"`
	Ordinal            uint32          `json:"ordinal"`
	ModelOperationID   string          `json:"model_operation_id"`
	ProviderCallID     string          `json:"provider_call_id"`
	Scope              AgentToolScope  `json:"scope"`
	Arguments          json.RawMessage `json:"arguments"`
	ArgumentsSHA256    string          `json:"arguments_sha256"`
}

type AgentTurnKey struct {
	CoreConversationID string `json:"core_conversation_id"`
	ClientTurnID       string `json:"client_turn_id"`
}
type AgentTurnStart struct {
	Key  AgentTurnKey `json:"key"`
	Text string       `json:"text"`
}
type AgentTurnPhase string

const (
	AgentTurnNew           AgentTurnPhase = "new"
	AgentTurnRunning       AgentTurnPhase = "model_running"
	AgentTurnAwaitingTools AgentTurnPhase = "tools_pending"
	AgentTurnCompleted     AgentTurnPhase = "completed"
	AgentTurnFailed        AgentTurnPhase = "rejected"
	AgentTurnUnknown       AgentTurnPhase = "unknown"
	AgentTurnExpired       AgentTurnPhase = "expired"
)

type AgentTurnSnapshot struct {
	Key                AgentTurnKey    `json:"key"`
	OwnerUserID        int64           `json:"owner_user_id"`
	CoreOwnerAccountID int64           `json:"core_owner_account_id"`
	Account            access.Account  `json:"account"`
	OriginalTextSHA256 string          `json:"original_text_sha256"`
	Revision           uint64          `json:"revision"`
	Round              uint32          `json:"round"`
	MaxRounds          uint32          `json:"max_rounds"`
	Phase              AgentTurnPhase  `json:"phase"`
	BudgetCredits      int64           `json:"budget_credits"`
	HeldCredits        int64           `json:"held_credits"`
	SettledCredits     int64           `json:"settled_credits"`
	PendingTools       []AgentToolCall `json:"pending_tools"`
	FinalText          string          `json:"final_text,omitempty"`
}
type AgentToolClaimInput struct {
	Key         AgentTurnKey `json:"key"`
	OperationID string       `json:"operation_id"`
	ClaimKey    string       `json:"claim_key"`
}
type AgentToolClaim struct {
	Call     AgentToolCall `json:"call"`
	ClaimKey string        `json:"claim_key"`
	// True is returned only for the fresh durable Core claim. Repeating a
	// claim after any lost ACK returns false and permits Inspect only.
	MayExecute     bool              `json:"may_execute"`
	AcceptedResult *AgentToolReceipt `json:"accepted_result,omitempty"`
}
type AgentToolCompletion struct {
	Claim  AgentToolClaimInput `json:"claim"`
	Result AgentToolResult     `json:"result"`
}
type AgentToolReceipt struct {
	OperationID     string         `json:"operation_id"`
	ClaimKey        string         `json:"claim_key"`
	State           AgentToolState `json:"state"`
	ResultSHA256    string         `json:"result_sha256"`
	DomainReceiptID string         `json:"domain_receipt_id"`
}

// The original Session is metadata, never a request actor/payer flag. All
// provider rounds and Core-domain tools execute durably inside Core. This port
// exposes only pending extension calls; repeated claims cannot authorize a new
// effect and completed results never create model or financial authority.
type AgentTurnPort interface {
	StartAgentTurn(context.Context, string, AgentTurnStart) (AgentTurnSnapshot, error)
	GetAgentTurn(context.Context, string, AgentTurnKey) (AgentTurnSnapshot, error)
	ClaimAgentTool(context.Context, string, AgentToolClaimInput) (AgentToolClaim, error)
	CompleteAgentTool(context.Context, string, AgentToolCompletion) (AgentToolReceipt, error)
}

// ChatPort is the shared host's transport-neutral chat boundary. Optional gift
// support is discovered separately; neither implementation can widen Core scope.
type ChatPort interface {
	Chat(context.Context, string, PersonalChatRequest) (PersonalChatReply, error)
	Recover(context.Context, string, int64, string) (PersonalChatReply, error)
}

// Credential and Actor are supplied by the consumer after fresh Core Session
// authorization, not parsed from model arguments or tool output. OriginalUserText
// comes from the immutable authenticated Core turn, never a claimed model intent.
type AgentInvocation struct {
	Credential       string
	Actor            access.Principal
	Call             AgentToolCall
	OriginalUserText string
}

type AgentToolState string

const (
	AgentToolSucceeded AgentToolState = "succeeded"
	AgentToolFailed    AgentToolState = "failed"
	AgentToolPending   AgentToolState = "pending"
	AgentToolUnknown   AgentToolState = "unknown"
)

// ResultJSON is bounded untrusted data. DomainReceiptID references the original
// durable business/financial operation; a string cannot authorize a Core effect.
// FinalText is used only by the trusted end_conversation adapter for this turn.
type AgentToolResult struct {
	OperationID     string          `json:"operation_id"`
	State           AgentToolState  `json:"state"`
	ResultJSON      json.RawMessage `json:"result_json"`
	ResultSHA256    string          `json:"result_sha256"`
	DomainReceiptID string          `json:"domain_receipt_id,omitempty"`
	FinalText       string          `json:"final_text,omitempty"`
}

// Effectful adapters use Call.OperationID as their immutable idempotency key.
// Inspect never starts an effect, issues a replacement grant or changes price.
// Core ownership/permission/financial checks still apply inside each domain.
type AgentToolExecutor interface {
	Execute(context.Context, AgentInvocation) (AgentToolResult, error)
	Inspect(context.Context, AgentInvocation) (AgentToolResult, error)
}
