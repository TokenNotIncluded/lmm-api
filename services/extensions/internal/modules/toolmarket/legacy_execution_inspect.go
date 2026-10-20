// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package toolmarket

import (
	"context"
	"crypto/hmac"
	"strconv"
)

type legacyExplicitWebResumeKey struct{}

// LegacyInspectExecution observes the original operation without reserving,
// dispatching, resuming, settling or releasing it. In particular a browser GET
// after Core confirmation cannot initiate paid generation.
func (m *Module) LegacyInspectExecution(ctx context.Context, credential, account, id string) (LegacyExecution, error) {
	user, err := m.WebAuthorize(ctx, credential, account, "toolmarket.read")
	if err != nil {
		return LegacyExecution{}, err
	}
	var r LegacyExecution
	err = m.store.view(func(d database) error {
		var ok bool
		r, ok = d.LegacyExecutions[id]
		if !ok || r.Owner.User != strconv.FormatInt(user, 10) || r.Owner.Account != account {
			return ErrMissing
		}
		return nil
	})
	if err != nil {
		return LegacyExecution{}, err
	}
	port, ok := m.authority.(CoreLegacyCredits)
	if !ok || missingInterface(port) {
		return r, ErrUnavailable
	}
	if r.Core.ID == "" {
		return r, ErrPending
	}
	receipt, err := port.QueryLegacyCredits(ctx, credential, LegacyCreditID{Owner: r.Owner, CallID: r.Core.ID})
	if err != nil {
		if r.Builtin != nil && r.ResultDigest == "" {
			r.State = "unknown"
		}
		return r, ErrPending
	}
	if !legacyCallMatches(receipt, r) || receipt.ID != r.Core.ID {
		return r, ErrDenied
	}
	r.Core = receipt
	if r.Builtin != nil && r.ResultDigest == "" && receipt.DispatchID != "" && receipt.State != "released" && !missingInterface(m.builtinRuntime) {
		ctx, _, _, err = m.builtinRecoveryContext(ctx, r)
		if err != nil {
			return r, err
		}
		copy := *r.Builtin
		r.Builtin = &copy
		out, queryErr := m.builtinRuntime.QueryBuiltin(ctx, credential, legacyBuiltinInvocation(r, builtinPeerIP(ctx)))
		if queryErr != nil {
			r.State = "unknown"
			return r, ErrPending
		}
		if out.CallID != receipt.ID || len(out.Result) > legacyBuiltinResultLimit(r) || out.DownstreamOperationID != "" && copy.DownstreamOperationID != "" && out.DownstreamOperationID != copy.DownstreamOperationID {
			return r, ErrDenied
		}
		r.Builtin.PreviewResult = out.Result
		r.Builtin.ResumeReady = out.ResumeReady
		if out.DownstreamOperationID != "" {
			r.Builtin.DownstreamOperationID = out.DownstreamOperationID
		}
		if len(out.Continuation) > 32768 {
			return r, ErrDenied
		}
		if len(out.Continuation) > 0 {
			r.Builtin.Continuation = out.Continuation
		}
		if out.AwaitingConfirmation || out.ResumeReady {
			r.State = "awaiting_confirmation"
			if r.Builtin.MCPRequestState == "" {
				r.Builtin.MCPRequestState = digest([]any{"builtin-mcp-state/v1", r.ID, r.InputDigest, r.Builtin.DownstreamOperationID, r.Builtin.Continuation})
			}
		}
	}
	if receipt.State == "settled" || receipt.State == "released" {
		r.State = "done"
		if r.Builtin != nil && r.Builtin.ExternalResult {
			return m.legacyDeliverDrawing(ctx, credential, r)
		}
		if r.ResultExpires > 0 && r.ResultExpires <= m.now().Unix() {
			r.Result = nil
		}
		return r, nil
	}
	return r, ErrPending
}

// LegacyResumeWebExecution uses only the retained input and original opaque
// binding. No browser field can replace payer, grant, tool, version or arguments.
func (m *Module) LegacyResumeWebExecution(ctx context.Context, credential, account, id, state string, responses []byte) (LegacyExecution, error) {
	user, err := m.WebAuthorize(ctx, credential, account, "toolmarket.execute")
	if err != nil {
		return LegacyExecution{}, err
	}
	var r LegacyExecution
	err = m.store.view(func(d database) error {
		var ok bool
		r, ok = d.LegacyExecutions[id]
		if !ok || r.Owner.User != strconv.FormatInt(user, 10) || r.Owner.Account != account || r.Input.ClientID != "web-market" {
			return ErrMissing
		}
		return nil
	})
	if err != nil {
		return LegacyExecution{}, err
	}
	if len(state) != 64 || r.Builtin == nil {
		return LegacyExecution{}, ErrInvalid
	}
	if LegacyMCPRequestState(r) == "" {
		// A preview ACK may have been lost before Go retained its continuation.
		// Validate the read-only Core observation before this explicit POST may
		// restore it. The original operation is never invoked again.
		observed, inspectErr := m.LegacyInspectExecution(ctx, credential, account, id)
		if inspectErr != nil && inspectErr != ErrPending {
			return observed, inspectErr
		}
		if !hmac.Equal([]byte(state), []byte(LegacyMCPRequestState(observed))) {
			return LegacyExecution{}, ErrConflict
		}
		port, ok := m.authority.(CoreLegacyCredits)
		if !ok || missingInterface(port) {
			return r, ErrUnavailable
		}
		restored, restoreErr := m.legacyRecoverExecution(ctx, port, credential, r)
		if restoreErr != nil && restoreErr != ErrPending || restored.State == "done" {
			return restored, restoreErr
		}
	}
	return m.LegacyExecuteMCP(context.WithValue(ctx, legacyExplicitWebResumeKey{}, true), credential, account, r.Input, state, responses)
}
