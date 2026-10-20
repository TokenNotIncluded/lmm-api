// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package toolmarket

import "context"

// Filtering archived display records never turns local counters into financial
// facts. Every returned row comes from a current owner-scoped Core page.
func legacyVisiblePage[T any](ctx context.Context, offset, limit int, fetch func(int, int) ([]T, error), visible func(T) bool) ([]T, error) {
	if !validWebPage(offset, limit) {
		return nil, ErrInvalid
	}
	rows := make([]T, 0, limit)
	skipped := 0
	for cursor := 0; cursor <= 10000; cursor += 100 {
		if ctx.Err() != nil {
			return nil, ErrUnavailable
		}
		batch, err := fetch(cursor, 100)
		if err != nil {
			return nil, err
		}
		for _, row := range batch {
			if !visible(row) {
				continue
			}
			if skipped < offset {
				skipped++
				continue
			}
			rows = append(rows, row)
			if len(rows) == limit {
				return rows, nil
			}
		}
		if len(batch) < 100 {
			return rows, nil
		}
	}
	return nil, ErrLimit
}
func (m *Module) legacyWebMine(ctx context.Context, credential, account string, user int64, kind string, offset, limit int) (any, error) {
	switch kind {
	case "grants":
		var hidden map[string]bool
		if err := m.store.view(func(d database) error {
			hidden = map[string]bool{}
			for id, g := range legacyAccess(&d).Grants {
				if g.UserID == user && g.Hidden {
					hidden[id] = true
				}
			}
			return nil
		}); err != nil {
			return nil, err
		}
		records, err := legacyVisiblePage(ctx, offset, limit, func(o, l int) ([]CoreLegacyConsentReceipt, error) {
			page, err := m.LegacyListConsents(ctx, credential, account, o, l)
			return page.Rows, err
		}, func(r CoreLegacyConsentReceipt) bool { return !hidden[r.ID] })
		if err != nil {
			return nil, err
		}
		type row struct {
			LegacyConsentGrant
			ReservedQuota   int64 `json:"reserved_quota"`
			SpentQuota      int64 `json:"spent_quota"`
			ReservedCalls   int64 `json:"reserved_calls"`
			SuccessfulCalls int64 `json:"successful_calls"`
		}
		rows := make([]row, 0, len(records))
		for _, r := range records {
			rows = append(rows, row{legacyGrantFromReceipt(r, user), r.ReservedQuota, r.SpentQuota, r.ReservedCalls, r.SuccessfulCalls})
		}
		return rows, nil
	case "tokens":
		var hidden map[string]bool
		if err := m.store.view(func(d database) error {
			hidden = map[string]bool{}
			for id, t := range legacyAccess(&d).Tokens {
				if t.UserID == user && t.Hidden {
					hidden[id] = true
				}
			}
			return nil
		}); err != nil {
			return nil, err
		}
		records, err := legacyVisiblePage(ctx, offset, limit, func(o, l int) ([]CoreLegacyTokenReceipt, error) {
			page, err := m.LegacyListTokens(ctx, credential, account, o, l)
			return page.Rows, err
		}, func(r CoreLegacyTokenReceipt) bool { return !hidden[r.ID] })
		if err != nil {
			return nil, err
		}
		rows := make([]LegacyTokenRecord, 0, len(records))
		for _, r := range records {
			rows = append(rows, LegacyTokenRecord{ID: r.ID, UserID: user, ClientID: r.ClientID, CanInvoke: r.CanInvoke, CanManage: r.CanManage, CreatedAt: r.CreatedAt, ExpiresAt: r.ExpiresAt, RevokedAt: r.RevokedAt})
		}
		return rows, nil
	case "budgets":
		page, err := m.LegacyListBudgets(ctx, credential, account, offset, limit)
		if err != nil {
			return nil, err
		}
		type row struct {
			LegacyBudgetRecord
			ReservedQuota int64 `json:"reserved_quota"`
			SpentQuota    int64 `json:"spent_quota"`
		}
		rows := make([]row, 0, len(page.Rows))
		for _, r := range page.Rows {
			rows = append(rows, row{LegacyBudgetRecord{UserID: user, Scope: r.Scope, ScopeID: r.ScopeID, LimitQuota: r.LimitQuota}, r.ReservedQuota, r.SpentQuota})
		}
		return rows, nil
	default:
		return nil, ErrMissing
	}
}
