// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
// Package assistant_market adapts authenticated Core tool claims to the existing
// market module. It owns no wallet, grant, remote transport or operation journal.
package assistant_market

import (
	"bytes"
	"context"
	"crypto/sha256"
	_ "embed"
	"encoding/hex"
	"encoding/json"
	"errors"
	"strconv"

	"github.com/TokenNotIncluded/lmm-api/services/extensions/internal/modules/assistant"
	"github.com/TokenNotIncluded/lmm-api/services/extensions/internal/modules/assistant/access"
	"github.com/TokenNotIncluded/lmm-api/services/extensions/internal/modules/assistant/redaction"
	"github.com/TokenNotIncluded/lmm-api/services/extensions/internal/modules/toolmarket"
)

const ClientID = "builtin-assistant"

//go:embed definition.json
var definitionJSON []byte

type definition struct {
	Name         string `json:"name"`
	Description  string `json:"description"`
	Version      string `json:"version"`
	Effect       string `json:"effect"`
	Access       string `json:"access"`
	SchemaJSON   string `json:"schema_json"`
	SchemaSHA256 string `json:"schema_sha256"`
}
type marketPort interface {
	WebAuthorize(context.Context, string, string, string) (int64, error)
	PublicWebCatalogue(context.Context, string, string, toolmarket.WebQuery) ([]toolmarket.WebSummary, error)
	PublicWebDetail(context.Context, string, string, string) (toolmarket.WebDetail, error)
	LegacyReadToolQuote(context.Context, string, string, string, string, string) (toolmarket.LegacyCreditQuote, error)
	LegacyExecuteRemote(context.Context, string, string, toolmarket.LegacyExecuteInput) (toolmarket.LegacyExecution, error)
	LegacyGetExecutionByRequestKey(context.Context, string, string, string, string) (toolmarket.LegacyExecution, error)
}
type Executor struct {
	market     marketPort
	definition definition
}

var _ assistant.AgentToolExecutor = (*Executor)(nil)

// New accepts the installed real module. A model/browser cannot inject a port.
func New(module *toolmarket.Module) (*Executor, error) {
	if module == nil {
		return nil, access.ErrInvalid
	}
	var d definition
	if json.Unmarshal(definitionJSON, &d) != nil || d.Name != "call_market_tool" || d.Version != assistant.FixedMainToolVersion || d.Effect != "server_guarded" || d.Access != "l1" || digest([]byte(d.SchemaJSON)) != d.SchemaSHA256 {
		return nil, access.ErrUnavailable
	}
	return &Executor{market: module, definition: d}, nil
}
func (e *Executor) Binding() assistant.AgentToolBinding {
	if e == nil {
		return assistant.AgentToolBinding{}
	}
	d := e.definition
	return assistant.AgentToolBinding{Name: d.Name, Description: d.Description, Version: d.Version, Effect: d.Effect, Access: d.Access, PriceVersion: assistant.BuiltinToolPriceVersion, PriceSHA256: assistant.FreeAgentPriceSHA256(), Parameters: json.RawMessage(d.SchemaJSON), Executor: e}
}
func digest(b []byte) string { h := sha256.Sum256(b); return hex.EncodeToString(h[:]) }

func validDigest(value string) bool {
	decoded, err := hex.DecodeString(value)
	return err == nil && len(decoded) == 32 && hex.EncodeToString(decoded) == value
}

type arguments struct {
	service, tool, version, grant string
	input                         json.RawMessage
}

func (e *Executor) validate(ctx context.Context, in assistant.AgentInvocation, inspect bool) (arguments, string, error) {
	var a arguments
	if e == nil || access.Missing(e.market) || in.Credential == "" || in.Actor.UserID <= 0 || in.Actor.Account != (access.Account{Kind: "personal", ID: in.Actor.UserID}) || len(in.Call.OperationID) == 0 || len(in.Call.OperationID) > 128 || digest(in.Call.Arguments) != in.Call.ArgumentsSHA256 {
		return a, "", access.ErrInvalid
	}
	s := in.Call.Scope
	d := e.definition
	if s.Name != d.Name || s.Version != d.Version || s.Domain != "extension" || s.SchemaSHA256 != d.SchemaSHA256 || s.PriceVersion != assistant.BuiltinToolPriceVersion || s.PriceSHA256 != assistant.FreeAgentPriceSHA256() || s.ResourceKind != "toolmarket" || !validDigest(s.ResourceSchemaSHA256) || !validDigest(s.ResourcePriceSHA256) {
		return a, "", access.ErrConflict
	}
	values, err := toolmarket.ValidateLegacyArguments(json.RawMessage(d.SchemaJSON), in.Call.Arguments)
	if err != nil {
		return a, "", access.ErrInvalid
	}
	a.service, _ = values["service_id"].(string)
	a.tool, _ = values["tool_id"].(string)
	a.version, _ = values["version_id"].(string)
	a.grant, _ = values["grant_id"].(string)
	a.input, err = json.Marshal(values["arguments"])
	if err != nil {
		return a, "", access.ErrInvalid
	}
	if a.tool != s.ResourceID || a.version != s.ResourceVersion || a.grant != s.GrantID {
		return a, "", access.ErrConflict
	}
	account := strconv.FormatInt(in.Actor.Account.ID, 10)
	permission := "toolmarket.execute"
	if inspect {
		permission = "toolmarket.read"
	}
	user, err := e.market.WebAuthorize(ctx, in.Credential, account, permission)
	if err != nil {
		return a, "", translate(err)
	}
	if user != in.Actor.UserID {
		return a, "", access.ErrForbidden
	}
	return a, account, nil
}
func (e *Executor) catalogue(ctx context.Context, in assistant.AgentInvocation, a arguments, account string) error {
	d, err := e.market.PublicWebDetail(ctx, in.Credential, account, a.service)
	if err != nil {
		return translate(err)
	}
	if d.Service.ID != a.service || d.Version.ServiceID != a.service || d.Version.ID != a.version || d.Service.LiveVersionID != a.version || d.Version.ExecutionType != "remote" || !d.Validated {
		return access.ErrConflict
	}
	var found *toolmarket.WebTool
	for i := range d.Tools {
		t := &d.Tools[i]
		if t.ToolID == a.tool {
			if found != nil {
				return access.ErrConflict
			}
			found = t
		}
	}
	if found == nil || found.VersionID != a.version {
		return access.ErrConflict
	}
	quote, err := e.market.LegacyReadToolQuote(ctx, in.Credential, account, a.service, a.version, a.tool)
	if err != nil {
		return translate(err)
	}
	if quote.ID == "" || quote.ServiceID != a.service || quote.VersionID != a.version || quote.ToolID != a.tool || quote.RemoteDigest != in.Call.Scope.ResourceSchemaSHA256 || quote.PricingDigest != in.Call.Scope.ResourcePriceSHA256 {
		return access.ErrConflict
	}
	price, err := toolmarket.LegacyQuotePricingDigest(found.LegacyPricing())
	if err != nil || price != quote.PricingDigest {
		return access.ErrConflict
	}
	if _, err = toolmarket.ValidateLegacyArguments(json.RawMessage(found.InputSchema), a.input); err != nil {
		return access.ErrInvalid
	}
	return nil
}
func (e *Executor) Execute(ctx context.Context, in assistant.AgentInvocation) (assistant.AgentToolResult, error) {
	a, account, err := e.validate(ctx, in, false)
	if err != nil {
		return assistant.AgentToolResult{}, err
	}
	if err = e.catalogue(ctx, in, a, account); err != nil {
		return assistant.AgentToolResult{}, err
	}
	record, err := e.market.LegacyExecuteRemote(ctx, in.Credential, account, toolmarket.LegacyExecuteInput{ClientID: ClientID, RequestKey: in.Call.OperationID, ToolID: a.tool, VersionID: a.version, GrantID: a.grant, Arguments: a.input})
	return e.outcome(in, a, account, record, err)
}
func (e *Executor) Inspect(ctx context.Context, in assistant.AgentInvocation) (assistant.AgentToolResult, error) {
	a, account, err := e.validate(ctx, in, true)
	if err != nil {
		return assistant.AgentToolResult{}, err
	}
	// Query only the original module operation. No catalogue change, missing
	// receipt or lookup failure can authorize LegacyExecuteRemote here.
	record, err := e.market.LegacyGetExecutionByRequestKey(ctx, in.Credential, account, ClientID, in.Call.OperationID)
	if errors.Is(err, toolmarket.ErrMissing) {
		return assistant.AgentResult(in.Call, assistant.AgentToolUnknown, map[string]any{"state": "unknown"}, "")
	}
	return e.outcome(in, a, account, record, err)
}
func (e *Executor) outcome(in assistant.AgentInvocation, a arguments, account string, r toolmarket.LegacyExecution, err error) (assistant.AgentToolResult, error) {
	if r.ID == "" {
		if err != nil {
			return assistant.AgentToolResult{}, translate(err)
		}
		return assistant.AgentToolResult{}, access.ErrUnavailable
	}
	if r.Owner.User != strconv.FormatInt(in.Actor.UserID, 10) || r.Owner.Account != account || r.Input.ClientID != ClientID || r.Input.RequestKey != in.Call.OperationID || r.Input.ToolID != a.tool || r.Input.VersionID != a.version || r.Input.GrantID != a.grant || !bytes.Equal(r.Input.Arguments, a.input) || r.MeteringBinding.ServiceID != a.service || r.MeteringBinding.VersionID != a.version || r.MeteringBinding.ToolID != a.tool || r.MeteringBinding.RemoteDigest != in.Call.Scope.ResourceSchemaSHA256 || r.SettlementPricingDigest != in.Call.Scope.ResourcePriceSHA256 {
		return assistant.AgentToolResult{}, access.ErrConflict
	}
	// Inspect relies on the original immutable input/economics and Core receipt;
	// a changed live catalogue must not create a new execution or replacement grant.
	state := assistant.AgentToolUnknown
	if r.State == "prepared" || r.State == "reserved" || r.State == "result" {
		state = assistant.AgentToolPending
	}
	payload := map[string]any{}
	receipt := ""
	if r.State == "done" && err == nil && (r.Core.State == "settled" || r.Core.State == "released") {
		if r.Core.ID == "" || r.Core.Owner != r.Owner || r.Core.ClientID != ClientID || r.Core.RequestKey != in.Call.OperationID || r.Core.ToolID != a.tool || r.Core.VersionID != a.version || r.Core.GrantID != a.grant || r.Core.PricingDigest != in.Call.Scope.ResourcePriceSHA256 {
			return assistant.AgentToolResult{}, access.ErrConflict
		}
		state = assistant.AgentToolFailed
		receipt = r.Core.ID
		if r.Core.State == "settled" && r.Success {
			if len(r.Result) == 0 || digest(r.Result) != r.ResultDigest {
				return assistant.AgentResult(in.Call, assistant.AgentToolUnknown, map[string]any{"state": "unknown"}, "")
			}
			state = assistant.AgentToolSucceeded
		}
		if len(r.Result) > 0 {
			if digest(r.Result) != r.ResultDigest {
				return assistant.AgentToolResult{}, access.ErrConflict
			}
			clean, cleanErr := redaction.JSON(r.Result, in.Credential)
			if cleanErr != nil || len(clean) > 48<<10 {
				return assistant.AgentResult(in.Call, assistant.AgentToolUnknown, map[string]any{"state": "unknown"}, "")
			}
			payload["result"] = clean
		}
	}
	if err != nil && !errors.Is(err, toolmarket.ErrPending) && !errors.Is(err, toolmarket.ErrUnavailable) {
		return assistant.AgentToolResult{}, translate(err)
	}
	payload["state"] = state
	return assistant.AgentResult(in.Call, state, payload, receipt)
}
func translate(err error) error {
	switch {
	case errors.Is(err, toolmarket.ErrInvalid):
		return access.ErrInvalid
	case errors.Is(err, toolmarket.ErrDenied), errors.Is(err, toolmarket.ErrConsent), errors.Is(err, toolmarket.ErrAuth):
		return access.ErrForbidden
	case errors.Is(err, toolmarket.ErrMissing):
		return access.ErrNotFound
	case errors.Is(err, toolmarket.ErrConflict):
		return access.ErrConflict
	case errors.Is(err, toolmarket.ErrLimit):
		return access.ErrLimit
	default:
		return access.ErrUnavailable
	}
}
