package bounties

import "context"

// QueryFunds is deliberately independent of Funds: recovery is a read operation
// and may never fall back to Preview or Execute on an older implementation.
type QueryFunds interface {
	Query(context.Context, string, string) (Receipt, error)
}

// QueryOperation authenticates even when a complete local receipt exists. An
// unknown/restarted operation is observed in Core without repeating its action.
func (s *Service) QueryOperation(ctx context.Context, session, operationID string) (Receipt, error) {
	if s == nil || s.Store == nil || s.Funds == nil || session == "" {
		return Receipt{}, ErrForbidden
	}
	if !validID(operationID) {
		return Receipt{}, ErrInvalid
	}
	actor, e := s.sessionActor(ctx, session)
	if e != nil {
		return Receipt{}, e
	}
	if !validID(actor.UserID) {
		return Receipt{}, ErrForbidden
	}
	var local operation
	e = s.Store.Within(ctx, func(tx Tx) error { return tx.Get("operations", operationID, &local) })
	if e == nil {
		if local.Actor != actor.UserID {
			return Receipt{}, ErrForbidden
		}
		if local.Result.Receipt != nil {
			r := *local.Result.Receipt
			if r.OperationID != operationID || r.ActorUserID != actor.UserID {
				return Receipt{}, ErrConflict
			}
			return r, nil
		}
		if p := local.Result.Project; p != nil {
			return Receipt{OperationID: operationID, ProjectID: p.ID, ActorUserID: actor.UserID, State: p.Status, Status: p.Status}, nil
		}
		return Receipt{}, ErrConflict
	}
	if e != ErrNotFound {
		return Receipt{}, e
	}
	q, ok := s.Funds.(QueryFunds)
	if !ok {
		return Receipt{}, ErrUnavailable
	}
	r, e := q.Query(ctx, session, operationID)
	if e != nil {
		return Receipt{}, e
	}
	if r.OperationID != operationID || r.ActorUserID != actor.UserID {
		return Receipt{}, ErrForbidden
	}
	return r, nil
}

// RecoveryFunds asks Core to compare every field of the original persisted
// command. Comparing receipt IDs alone cannot authorize evidence projections.
type RecoveryFunds interface {
	QueryForCommand(context.Context, string, Command) (Receipt, error)
}

func pendingReceipt(r Receipt) bool {
	switch r.State {
	case "awaiting_confirmation", "confirmed", "confirmation_expired":
		return true
	}
	return false
}

// SessionFunds verifies current native Session without granting developer-only
// new-resource authority. Core still checks frozen ownership on every action.
type SessionFunds interface {
	SessionAuthorize(context.Context, string) (Actor, error)
}

func (s *Service) sessionActor(ctx context.Context, token string) (Actor, error) {
	if f, ok := s.Funds.(SessionFunds); ok {
		return f.SessionAuthorize(ctx, token)
	}
	return s.Funds.Authorize(ctx, token)
}
func protectedAction(action string) bool {
	switch action {
	case "close", "approve", "reject", "withdraw", "cancel", "submit", "open_dispute", "resolve_dispute":
		return true
	}
	return false
}
