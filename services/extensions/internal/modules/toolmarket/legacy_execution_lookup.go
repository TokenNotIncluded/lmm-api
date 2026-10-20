// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package toolmarket

import "context"

// LegacyGetExecutionByRequestKey inspects an original operation after a lost
// acknowledgement. It never creates an operation or dispatches a remote call.
func (m *Module) LegacyGetExecutionByRequestKey(ctx context.Context, credential, account, clientID, requestKey string) (LegacyExecution, error) {
	user, err := m.WebAuthorize(ctx, credential, account, "toolmarket.read")
	if err != nil {
		return LegacyExecution{}, err
	}
	if !legacyClientValid(clientID) || len(requestKey) == 0 || len(requestKey) > 128 {
		return LegacyExecution{}, ErrInvalid
	}
	return m.LegacyInspectExecution(ctx, credential, account, digest([]any{user, clientID, requestKey}))
}
