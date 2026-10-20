// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package toolmarket

import (
	"context"
	"encoding/json"
)

// Only the immutable compiled generation tool can use the Core image envelope
// bound. Remote tools and other builtins retain their original 2 MiB limit.
const LegacyDrawingResultLimit = (32 << 20) + (128 << 10)

func legacyDrawingExecution(r LegacyExecution) bool {
	if r.Builtin == nil || r.Builtin.ServiceKey != "drawing" || r.Builtin.ToolName != "drawing.generate" {
		return false
	}
	definition, entry, err := compiledBuiltin(r.Builtin.ServiceKey, r.Builtin.ToolName)
	return err == nil && definition.VersionID == r.Input.VersionID && entry.ToolID == r.Input.ToolID && entry.InputSchemaDigest == r.Builtin.InputSchemaDigest && r.Pricing.BillingMode == "" && r.Pricing.PriceQuota == 0 && r.Core.Held == 0
}
func legacyBuiltinResultLimit(r LegacyExecution) int {
	if legacyDrawingExecution(r) {
		return LegacyDrawingResultLimit
	}
	return 2 << 20
}
func legacyDrawingResult(r LegacyExecution, out BuiltinResult, schema string) (json.RawMessage, error) {
	if !legacyDrawingExecution(r) || out.CallID != r.Core.ID || out.ExecutionStatus != "completed" || out.SettlementStatus != "settled" || out.AwaitingConfirmation || out.ResumeReady || out.DownstreamOperationID == "" || out.DownstreamOperationID != r.Builtin.DownstreamOperationID || len(out.Result) == 0 || len(out.Result) > LegacyDrawingResultLimit {
		return nil, ErrDenied
	}
	var envelope map[string]any
	if strictLegacyJSON(out.Result, &envelope, false) != nil || !legacyExactBounded(envelope, 0) {
		return nil, ErrDenied
	}
	if isError, exists := envelope["isError"]; exists && isError != false {
		return nil, ErrDenied
	}
	if envelope["requestState"] != nil || envelope["inputRequests"] != nil {
		return nil, ErrDenied
	}
	document, ok := envelope["structuredContent"].(map[string]any)
	if !ok || document["operation_id"] != out.DownstreamOperationID || document["state"] != "completed" || document["billing_pending"] != false || document["output"] == nil {
		return nil, ErrDenied
	}
	charged, ok := document["charged_credits"].(json.Number)
	if !ok {
		return nil, ErrDenied
	}
	n, err := charged.Int64()
	if err != nil || n < 0 {
		return nil, ErrDenied
	}
	compiled, err := legacySchema(json.RawMessage(schema))
	if err != nil || compiled.Validate(document) != nil {
		return nil, ErrDenied
	}
	// This digest authenticates delivery from the original Core operation. The
	// downstream model subsystem owns the paid image receipt; market price is zero.
	clean, err := json.Marshal(envelope)
	if err != nil {
		return nil, ErrDenied
	}
	return clean, nil
}
func (m *Module) legacyFinishDrawing(ctx context.Context, port CoreLegacyCredits, credential string, r LegacyExecution, tool WebTool, out BuiltinResult) (LegacyExecution, error) {
	clean, err := legacyDrawingResult(r, out, tool.OutputSchema)
	if err != nil {
		return r, err
	}
	r.Builtin.ExternalResult = true
	r.Builtin.PreviewResult = nil
	r.Result, r.RawResult = nil, nil
	r.ResultDigest = digest(clean)
	r.AuthorityResultDigest = r.ResultDigest
	r.Success, r.State = true, "result"
	r.ResultExpires = 0 // Core retains the paid output; delivery is queried there.
	if m.legacySave(r) != nil {
		return r, ErrPending
	}
	return m.legacyRecoverExecution(ctx, port, credential, r)
}
func (m *Module) legacyDeliverDrawing(ctx context.Context, credential string, r LegacyExecution) (LegacyExecution, error) {
	if !legacyDrawingExecution(r) || r.State != "done" || r.Core.State != "settled" || credential == "" || missingInterface(m.builtinRuntime) {
		return r, ErrPending
	}
	_, entry, err := compiledBuiltin(r.Builtin.ServiceKey, r.Builtin.ToolName)
	if err != nil {
		return r, ErrDenied
	}
	out, err := m.builtinRuntime.QueryBuiltin(ctx, credential, legacyBuiltinInvocation(r, builtinPeerIP(ctx)))
	if err != nil {
		return r, ErrPending
	}
	clean, err := legacyDrawingResult(r, out, entry.OutputSchema)
	if err != nil || digest(clean) != r.ResultDigest {
		return r, ErrDenied
	}
	// Ephemeral only: never write the image bytes into the shared Go snapshot.
	r.Result = clean
	return r, nil
}
