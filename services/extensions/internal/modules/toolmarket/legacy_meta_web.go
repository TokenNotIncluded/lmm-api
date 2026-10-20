// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package toolmarket

import "context"

func (m *Module) legacyWebMetaDelegation(ctx context.Context, credential, account string, in WebRequest) (any, error) {
	if in.Action == "delegation.get" {
		return m.LegacyGetMetaDelegation(ctx, credential, account, in.Kind, in.ID)
	}
	if in.Action != "delegation.set" {
		return nil, ErrMissing
	}
	if len(in.Body) > 1024 {
		return nil, ErrInvalid
	}
	value, err := payload[struct {
		Enabled       bool  `json:"enabled"`
		MaxTotalQuota int64 `json:"max_total_quota"`
		ExpiresAt     int64 `json:"expires_at"`
	}](in.Body, "enabled", "max_total_quota")
	if err != nil {
		return nil, err
	}
	operation := in.OperationID
	if operation == "" {
		operation, err = randomID()
		if err != nil {
			return nil, err
		}
	}
	return m.LegacySetMetaDelegation(ctx, credential, account, LegacyMarketMetaSet{OperationID: operation, Kind: in.Kind, ID: in.ID, Enabled: value.Enabled, MaxTotalQuota: value.MaxTotalQuota, ExpiresAt: value.ExpiresAt})
}
