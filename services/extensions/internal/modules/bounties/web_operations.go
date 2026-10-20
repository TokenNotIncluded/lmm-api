package bounties

import (
	"context"
	"crypto/rand"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
)

type WebOperation struct {
	ID          string     `json:"id"`
	ActorUserID string     `json:"actor_user_id"`
	Hash        string     `json:"hash"`
	Invocation  Invocation `json:"invocation"`
	State       string     `json:"state"`
	Result      Result     `json:"result"`
	CreatedAt   int64      `json:"created_at"`
}

func randomID() (string, error) {
	var b [24]byte
	if _, e := rand.Read(b[:]); e != nil {
		return "", ErrUnavailable
	}
	return hex.EncodeToString(b[:]), nil
}
func digestID(raw string) string { b := sha256.Sum256([]byte(raw)); return hex.EncodeToString(b[:]) }
func (s *Service) webActor(ctx context.Context, session, action string) (Actor, error) {
	if protectedAction(action) {
		return s.sessionActor(ctx, session)
	}
	return s.Funds.Authorize(ctx, session)
}
func localAction(action string) bool { return action == "create_draft" || action == "update_draft" }

// PrepareWeb stores the original command before any Core preview. The session
// credential is never retained. A restarted financial preparation queries only.
func (s *Service) PrepareWeb(ctx context.Context, session, key string, in Invocation) (WebOperation, error) {
	var out WebOperation
	actor, e := s.webActor(ctx, session, in.Command.Action)
	if e != nil {
		return out, e
	}
	if key != "" && !validID(key) {
		return out, ErrInvalid
	}
	id := ""
	if requested := in.Command.OperationID; requested != "" {
		if !validID(requested) || key != "" && key != requested {
			return out, ErrInvalid
		}
		id = requested
	} else if key != "" {
		id = digestID(actor.UserID + ":" + key)
	} else {
		id, e = randomID()
		if e != nil {
			return out, e
		}
	}
	in.Actor = Actor{}
	in.Command.OperationID = id
	in.Command.Confirmation = ""
	in.Preview = false
	if in.Command.Action == "create_draft" && in.Command.ProjectID == "" {
		in.Command.ProjectID = digestID(id + ":project")
	}
	if in.Command.Action == "accept" && in.Command.ChallengeID == "" {
		in.Command.ChallengeID = digestID(id + ":challenge")
	}
	if in.Command.Action == "open_dispute" && in.Command.DisputeID == "" {
		in.Command.DisputeID = digestID(id + ":dispute")
	}
	raw, _ := json.Marshal(in)
	hash := digestID(string(raw))
	created := false
	e = s.Store.Within(ctx, func(tx Tx) error {
		e := tx.Get("web_operations", id, &out)
		if e == nil {
			if out.ActorUserID != actor.UserID || out.Hash != hash {
				return ErrConflict
			}
			return nil
		}
		if e != ErrNotFound {
			return e
		}
		created = true
		out = WebOperation{ID: id, ActorUserID: actor.UserID, Hash: hash, Invocation: in, State: "preparing", CreatedAt: s.now()}
		return tx.Put("web_operations", id, out)
	})
	if e != nil {
		return WebOperation{}, e
	}
	if !created && out.State != "preparing" {
		return out, nil
	}
	var result Result
	if localAction(in.Command.Action) {
		in.Actor.Token = session
		result, e = s.BuiltinInvoke(ctx, in)
	} else if created {
		in.Actor.Token = session
		in.Preview = true
		result, e = s.BuiltinInvoke(ctx, in)
	} else {
		result, e = s.RecoverOperation(ctx, session, in)
	}
	if e != nil {
		return out, e
	}
	out.Result = result
	if localAction(in.Command.Action) {
		out.State = "settled"
	} else if result.Receipt != nil {
		out.State = result.Receipt.State
		out.Invocation.Command.Confirmation = result.Receipt.Confirmation
	} else {
		return out, ErrConflict
	}
	e = s.Store.Within(ctx, func(tx Tx) error {
		var current WebOperation
		if e := tx.Get("web_operations", id, &current); e != nil {
			return e
		}
		if current.State != "preparing" {
			out = current
			return nil
		}
		return tx.Put("web_operations", id, out)
	})
	return out, e
}
func (s *Service) webOperation(ctx context.Context, session, id string) (WebOperation, error) {
	var out WebOperation
	if !validID(id) {
		return out, ErrInvalid
	}
	actor, e := s.sessionActor(ctx, session)
	if e != nil {
		return out, e
	}
	e = s.Store.Within(ctx, func(tx Tx) error {
		if e := tx.Get("web_operations", id, &out); e != nil {
			return e
		}
		if out.ActorUserID != actor.UserID {
			return ErrForbidden
		}
		return nil
	})
	return out, e
}
func (s *Service) ApplyWeb(ctx context.Context, session, id string, recoverOnly bool) (WebOperation, error) {
	op, e := s.webOperation(ctx, session, id)
	if e != nil {
		return op, e
	}
	if op.State == "settled" {
		return op, nil
	}
	// The Core-confirmed request is checked before winning the unique dispatch
	// slot. A caller cannot bypass browser proof by posting apply directly.
	confirmed := false
	if !recoverOnly && (op.State == "preparing" || op.State == "awaiting_confirmation" || op.State == "confirmed") {
		proof, err := s.RecoverOperation(ctx, session, op.Invocation)
		if err != nil {
			return op, err
		}
		if proof.Receipt == nil {
			return op, ErrConflict
		}
		if !pendingReceipt(*proof.Receipt) {
			op.Result = proof
			op.State = "settled"
			err = s.Store.Within(ctx, func(tx Tx) error { return tx.Put("web_operations", id, op) })
			return op, err
		}
		if proof.Receipt.State != "confirmed" {
			op.Result = proof
			op.State = proof.Receipt.State
			op.Invocation.Command.Confirmation = proof.Receipt.Confirmation
			err = s.Store.Within(ctx, func(tx Tx) error {
				var current WebOperation
				if e := tx.Get("web_operations", id, &current); e != nil {
					return e
				}
				if current.State == "dispatching" || current.State == "unknown" || current.State == "settled" {
					op = current
					return nil
				}
				return tx.Put("web_operations", id, op)
			})
			return op, err
		}
		confirmed = true
		op.Invocation.Command.Confirmation = proof.Receipt.Confirmation
	}
	invoke := false
	confirmation := op.Invocation.Command.Confirmation
	e = s.Store.Within(ctx, func(tx Tx) error {
		if e := tx.Get("web_operations", id, &op); e != nil {
			return e
		}
		if op.State == "settled" {
			return nil
		}
		if confirmed && !recoverOnly && (op.State == "preparing" || op.State == "awaiting_confirmation" || op.State == "confirmed") {
			op.Invocation.Command.Confirmation = confirmation
			invoke = true
			op.State = "dispatching"
			return tx.Put("web_operations", id, op)
		}
		return nil
	})
	if e != nil {
		return op, e
	}
	if op.State == "settled" {
		return op, nil
	}
	var result Result
	if invoke {
		in := op.Invocation
		in.Actor.Token = session
		result, e = s.BuiltinInvoke(ctx, in)
	} else {
		result, e = s.RecoverOperation(ctx, session, op.Invocation)
	}
	if e != nil {
		if invoke {
			op.State = "unknown"
			if e == ErrForbidden || e == ErrInvalid || e == ErrNotFound || e == ErrConflict {
				op.State = "awaiting_confirmation"
			}
			_ = s.Store.Within(ctx, func(tx Tx) error {
				var current WebOperation
				if e := tx.Get("web_operations", id, &current); e != nil {
					return e
				}
				if current.State == "settled" {
					op = current
					return nil
				}
				return tx.Put("web_operations", id, op)
			})
		}
		return op, e
	}
	neverDispatched := op.State == "preparing" || op.State == "awaiting_confirmation" || op.State == "confirmed" || op.State == "confirmation_expired"
	op.Result = result
	if result.Receipt != nil && pendingReceipt(*result.Receipt) {
		if neverDispatched {
			op.State = result.Receipt.State
			op.Invocation.Command.Confirmation = result.Receipt.Confirmation
		} else {
			op.State = "unknown"
		}
	} else {
		op.State = "settled"
	}
	e = s.Store.Within(ctx, func(tx Tx) error {
		var current WebOperation
		if e := tx.Get("web_operations", id, &current); e != nil {
			return e
		}
		if current.State == "settled" {
			op = current
			return nil
		}
		return tx.Put("web_operations", id, op)
	})
	return op, e
}
