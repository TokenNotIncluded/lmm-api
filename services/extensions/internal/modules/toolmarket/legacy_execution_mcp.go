// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package toolmarket

import (
	"context"
	"crypto/hmac"
	"encoding/json"
)

// LegacyMCPRequestState is an opaque binding to the retained original operation,
// never a human confirmation or a replacement downstream authorization token.
func LegacyMCPRequestState(r LegacyExecution) string {
	if r.Builtin == nil {
		return ""
	}
	return r.Builtin.MCPRequestState
}

// MCP envelope state/responses do not enter business arguments or RequestHash.
// Confirmation authority is observed from the original domain Core operation;
// a caller's InputResponses cannot mark an operation confirmed.
func (m *Module) LegacyExecuteMCP(ctx context.Context, credential, account string, in LegacyExecuteInput, requestState string, inputResponses json.RawMessage) (LegacyExecution, error) {
	if requestState == "" && len(inputResponses) == 0 {
		return m.LegacyExecute(ctx, credential, account, in)
	}
	if len(requestState) != 64 || len(inputResponses) > 32768 {
		return LegacyExecution{}, ErrInvalid
	}
	if len(inputResponses) > 0 {
		var response any
		if strictLegacyJSON(inputResponses, &response, false) != nil {
			return LegacyExecution{}, ErrInvalid
		}
	}
	user, e := m.WebAuthorize(ctx, credential, account, "toolmarket.execute")
	if e != nil {
		return LegacyExecution{}, e
	}
	var args any
	if decodeStrict(in.Arguments, &args) != nil {
		return LegacyExecution{}, ErrInvalid
	}
	in.Arguments, _ = json.Marshal(args)
	id := digest([]any{user, in.ClientID, in.RequestKey})
	var r LegacyExecution
	e = m.store.view(func(d database) error {
		var found bool
		r, found = d.LegacyExecutions[id]
		if !found || r.Owner.Account != account || r.RequestHash != digest(in) || r.Builtin == nil || !hmac.Equal([]byte(LegacyMCPRequestState(r)), []byte(requestState)) {
			return ErrConflict
		}
		return nil
	})
	if e != nil {
		return LegacyExecution{}, e
	}
	port, ok := m.authority.(CoreLegacyCredits)
	if !ok || missingInterface(port) {
		return r, ErrUnavailable
	}
	return m.legacyRecoverExecution(ctx, port, credential, r)
}
