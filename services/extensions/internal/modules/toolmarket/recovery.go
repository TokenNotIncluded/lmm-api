// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package toolmarket

import "context"

// RecoverSettlements completes only saved native results against their original
// Core hold. It never dispatches a tool or needs a live logged-in user to finish
// an already-authorized consumption. Unknown external effects remain pending.
func (m *Module) RecoverSettlements(ctx context.Context, limit int) (int, error) {
	if m == nil || limit < 1 || limit > 20 {
		return 0, ErrInvalid
	}
	if _, ok := m.funds.(ResultFunds); !ok {
		return 0, ErrFunds
	}
	type pending struct {
		id     string
		record Execution
	}
	var work []pending
	if err := m.store.view(func(d database) error {
		for id, r := range d.Executions {
			if r.State == "settlement_pending" {
				work = append(work, pending{id, r})
				if len(work) == limit {
					break
				}
			}
		}
		return nil
	}); err != nil {
		return 0, err
	}
	completed := 0
	for _, item := range work {
		if ctx.Err() != nil {
			return completed, ErrUnavailable
		}
		if _, err := m.settle(ctx, item.record.Owner, "", item.id, item.record); err != nil {
			return completed, err
		}
		completed++
	}
	return completed, nil
}
