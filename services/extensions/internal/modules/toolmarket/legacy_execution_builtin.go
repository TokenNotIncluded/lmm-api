// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package toolmarket

import (
	"context"
	"encoding/json"
)

// Invoking and observing an existing operation are deliberately separate.
// UNKNOWN and awaiting confirmation must never repeat InvokeBuiltin.
type LegacyBuiltinRuntime interface {
	InvokeBuiltin(context.Context, string, BuiltinInvocation) (BuiltinResult, error)
	QueryBuiltin(context.Context, string, BuiltinInvocation) (BuiltinResult, error)
}

// Resume is a separate one-use operation after authoritative confirmation and a
// durable claim. Its presence alone does not authorize an automatic resume.
type LegacyBuiltinResumer interface {
	ResumeBuiltin(context.Context, string, BuiltinInvocation) (BuiltinResult, error)
}

// Only these immutable compiled identities can select internal authority.
type LegacyBuiltinExecution struct {
	ServiceKey            string          `json:"service_key"`
	ToolName              string          `json:"tool_name"`
	InputSchemaDigest     string          `json:"input_schema_digest"`
	DownstreamOperationID string          `json:"downstream_operation_id,omitempty"`
	Continuation          json.RawMessage `json:"continuation,omitempty"`
	PreviewResult         json.RawMessage `json:"preview_result,omitempty"`
	ResumeReady           bool            `json:"resume_ready"`
	Indeterminate         bool            `json:"indeterminate"`
	MCPRequestState       string          `json:"mcp_request_state,omitempty"`
	ExternalResult        bool            `json:"external_result"`
	ResumeClaimed         bool            `json:"resume_claimed"`
}

func legacyCompiledExecution(snapshot webSnapshot, tool WebTool) (*LegacyBuiltinExecution, error) {
	for _, definition := range CompiledBuiltinDefinitions() {
		if definition.ServiceID != snapshot.Version.ServiceID || definition.VersionID != snapshot.Version.ID || definition.Digest != snapshot.Digest {
			continue
		}
		for _, entry := range definition.Tools {
			if entry.ToolID == tool.ToolID && entry.Name == tool.Name && entry.InputSchemaDigest == snapshot.RemoteDigests[tool.ToolID] && sameWebSchema(entry.InputSchema, tool.InputSchema) && sameWebSchema(entry.OutputSchema, tool.OutputSchema) {
				if tool.PriceQuota != 0 || tool.BillingMode != "" || tool.ProviderPricing != nil {
					return nil, ErrConflict
				}
				return &LegacyBuiltinExecution{ServiceKey: definition.Key, ToolName: tool.Name, InputSchemaDigest: entry.InputSchemaDigest}, nil
			}
		}
	}
	return nil, ErrDenied
}
func legacyBuiltinInvocation(r LegacyExecution, peer string) BuiltinInvocation {
	return BuiltinInvocation{Context: BuiltinContext{CallID: r.Core.ID, DispatchID: r.Core.DispatchID, ToolID: r.Input.ToolID, VersionID: r.Input.VersionID, GrantID: r.Input.GrantID, InputDigest: r.InputDigest}, Owner: r.Owner, ServiceKey: r.Builtin.ServiceKey, ToolName: r.Builtin.ToolName, Arguments: r.Input.Arguments, Continuation: r.Builtin.Continuation, ActualPeerIP: peer}
}
func (m *Module) legacySaveBuiltinOutcome(ctx context.Context, port CoreLegacyCredits, credential string, r LegacyExecution, tool WebTool, out BuiltinResult) (LegacyExecution, error) {
	if r.Builtin == nil || out.CallID != r.Core.ID || len(out.Continuation) > 32768 || len(out.Result) > legacyBuiltinResultLimit(r) {
		return r, ErrDenied
	}
	if r.Builtin.DownstreamOperationID != "" && out.DownstreamOperationID != "" && r.Builtin.DownstreamOperationID != out.DownstreamOperationID {
		return r, ErrConflict
	}
	if out.DownstreamOperationID != "" {
		r.Builtin.DownstreamOperationID = out.DownstreamOperationID
	}
	if len(out.Continuation) > 0 {
		r.Builtin.Continuation = append(json.RawMessage(nil), out.Continuation...)
	}
	r.Builtin.ResumeReady = out.ResumeReady
	if out.ExecutionStatus == "completed" && out.SettlementStatus == "settled" && !out.AwaitingConfirmation && !out.ResumeReady {
		if legacyDrawingExecution(r) {
			return m.legacyFinishDrawing(ctx, port, credential, r, tool, out)
		}
		r.Builtin.PreviewResult = nil
		if e := m.legacySave(r); e != nil {
			return r, ErrPending
		}
		return m.legacyFinishRemote(ctx, port, r, tool, out.Result, credential)
	}
	r.Builtin.PreviewResult = append(json.RawMessage(nil), out.Result...)
	if len(r.Builtin.PreviewResult) > 2<<20 {
		r.Builtin.PreviewResult = nil
	}
	r.State = "unknown"
	if out.AwaitingConfirmation || out.ResumeReady {
		r.State = "awaiting_confirmation"
		if r.Builtin.MCPRequestState == "" {
			r.Builtin.MCPRequestState = digest([]any{"builtin-mcp-state/v1", r.ID, r.InputDigest, r.Builtin.DownstreamOperationID, r.Builtin.Continuation})
		}
	}
	if e := m.legacySave(r); e != nil {
		return r, ErrPending
	}
	return r, ErrPending
}
func (m *Module) legacyQueryBuiltin(ctx context.Context, port CoreLegacyCredits, credential string, r LegacyExecution) (LegacyExecution, error) {
	if r.Builtin == nil || missingInterface(m.builtinRuntime) || credential == "" || r.Core.DispatchID == "" {
		return r, ErrPending
	}
	ctx, _, entry, e := m.builtinRecoveryContext(ctx, r)
	if e != nil {
		return r, ErrDenied
	}
	priorState := r.State
	out, e := m.builtinRuntime.QueryBuiltin(ctx, credential, legacyBuiltinInvocation(r, builtinPeerIP(ctx)))
	if e != nil {
		r.State = "unknown"
		return r, ErrPending
	}
	observed, resultErr := m.legacySaveBuiltinOutcome(ctx, port, credential, r, entry.WebTool, out)
	explicit, _ := ctx.Value(legacyExplicitWebResumeKey{}).(bool)
	if resultErr != ErrPending || !out.ResumeReady || priorState != "awaiting_confirmation" && !explicit || observed.Builtin.ResumeClaimed || observed.Builtin.Indeterminate && !explicit {
		return observed, resultErr
	}
	resumer, ok := m.builtinRuntime.(LegacyBuiltinResumer)
	if !ok || missingInterface(resumer) {
		return observed, ErrPending
	}
	claimed := false
	e = m.store.update(func(d *database) error {
		current, ok := d.LegacyExecutions[r.ID]
		if !ok || current.Owner != r.Owner || current.RequestHash != r.RequestHash || current.Builtin == nil {
			return ErrConflict
		}
		if current.State != "awaiting_confirmation" && !(explicit && current.State == "unknown") || !current.Builtin.ResumeReady || current.Builtin.ResumeClaimed || current.Builtin.Indeterminate && !explicit {
			return nil
		}
		if current.Builtin.DownstreamOperationID != observed.Builtin.DownstreamOperationID || digest(current.Builtin.Continuation) != digest(observed.Builtin.Continuation) {
			return ErrConflict
		}
		current.Builtin.ResumeClaimed = true
		current.State = "unknown"
		d.LegacyExecutions[r.ID] = current
		observed = current
		claimed = true
		return nil
	})
	if e != nil || !claimed {
		return observed, ErrPending
	}
	// Domain Core execute rechecks the original confirmed operation, current
	// identity/grant/schema/input and performs its own durable single-winner claim.
	// This host CAS only prevents duplicate calls; it creates no spend authority.
	resumed, e := resumer.ResumeBuiltin(ctx, credential, legacyBuiltinInvocation(observed, builtinPeerIP(ctx)))
	if e != nil {
		observed.Builtin.Indeterminate = true
		_ = m.legacySave(observed)
		_, _ = port.UnknownLegacyCredits(ctx, LegacyCreditID{Owner: observed.Owner, CallID: observed.Core.ID, DispatchID: observed.Core.DispatchID})
		return observed, ErrPending
	}
	return m.legacySaveBuiltinOutcome(ctx, port, credential, observed, entry.WebTool, resumed)
}
