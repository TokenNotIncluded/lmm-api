// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package toolmarket

import "context"

// ReleaseFunds may release only an original Core reservation that has never
// crossed Core's dispatch boundary. A timeout alone is never release evidence.
type ReleaseFunds interface {
	ReleaseUndispatched(context.Context, string, Principal, Charge) error
}

func (m *Module) cancelExecution(ctx context.Context, p Principal, credential, key string) (any, error) {
	if !validID.MatchString(key) {
		return nil, ErrInvalid
	}
	if err := m.reauthorize(ctx, p, credential, "toolmarket.execute"); err != nil {
		return nil, err
	}
	id := digest([]string{"toolmarket/v1", p.key(), key})
	var record Execution
	if err := m.store.view(func(d database) error {
		r, ok := d.Executions[id]
		if !ok || r.Owner != p {
			return ErrMissing
		}
		record = r
		return nil
	}); err != nil {
		return nil, err
	}
	if record.State == "cancelled" {
		return map[string]string{"state": "cancelled"}, nil
	}
	if record.State != "started" && record.State != "dispatched" {
		return nil, ErrConflict
	}
	funds, ok := m.funds.(ReleaseFunds)
	if !ok {
		return nil, ErrFunds
	}
	// Core serializes release against dispatch. If dispatch wins or receipt is
	// absent/unknown, this must fail; the external tool is never called again.
	if err := funds.ReleaseUndispatched(ctx, credential, p, record.Charge); err != nil {
		return nil, err
	}
	if err := m.store.update(func(d *database) error {
		r, ok := d.Executions[id]
		if !ok || r.Owner != p || r.Fingerprint != record.Fingerprint || r.RequestHash != record.RequestHash || r.Charge != record.Charge {
			return ErrConflict
		}
		if r.State == "cancelled" {
			return nil
		}
		if r.State != "started" && r.State != "dispatched" {
			return ErrConflict
		}
		r.State = "cancelled"
		r.Resume = nil
		r.Result = nil
		d.Executions[id] = r
		d.audit(p, "execute.cancel", id, "released")
		return nil
	}); err != nil {
		return nil, ErrPending
	}
	return map[string]string{"state": "cancelled"}, nil
}
