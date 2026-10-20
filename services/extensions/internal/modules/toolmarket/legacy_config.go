// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package toolmarket

import "context"

// LegacyMarketConfig is an authoritative Core policy readback. It is not a
// local switch that can enable execution while the Core provider is unavailable.
type LegacyMarketConfig struct {
	Enabled     bool  `json:"enabled"`
	FeeBPS      int64 `json:"fee_bps"`
	RecipientID int64 `json:"recipient_id"`
	UpdatedBy   int64 `json:"updated_by"`
	UpdatedAt   int64 `json:"updated_at"`
	Version     int64 `json:"version"`
}
type LegacyMarketConfigUpdate struct {
	OperationID string `json:"operation_id"`
	Enabled     bool   `json:"enabled"`
	FeeBPS      int64  `json:"fee_bps"`
	RecipientID int64  `json:"recipient_id"`
}
type LegacyMarketConfigUpdateReceipt struct {
	LegacyMarketConfig
	OperationID string `json:"operation_id"`
}
type CoreLegacyConfig interface {
	ReadLegacyConfig(context.Context) (LegacyMarketConfig, error)
	UpdateLegacyConfig(context.Context, string, LegacyMarketConfigUpdate) (LegacyMarketConfigUpdateReceipt, error)
}

func (c *LegacyMarketConfig) UnmarshalJSON(raw []byte) error {
	type alias LegacyMarketConfig
	var next alias
	if err := decodeStrict(raw, &next); err != nil {
		return err
	}
	if err := legacyRequiredFields(raw, "enabled", "fee_bps", "recipient_id", "updated_by", "updated_at", "version"); err != nil {
		return err
	}
	*c = LegacyMarketConfig(next)
	return nil
}
func (r *LegacyMarketConfigUpdateReceipt) UnmarshalJSON(raw []byte) error {
	// An alias containing the embedded config would inherit its unmarshaler and
	// swallow operation_id. Decode an explicit closed wire shape instead.
	var next struct {
		Enabled     bool   `json:"enabled"`
		FeeBPS      int64  `json:"fee_bps"`
		RecipientID int64  `json:"recipient_id"`
		UpdatedBy   int64  `json:"updated_by"`
		UpdatedAt   int64  `json:"updated_at"`
		Version     int64  `json:"version"`
		OperationID string `json:"operation_id"`
	}
	if err := decodeStrict(raw, &next); err != nil {
		return err
	}
	if err := legacyRequiredFields(raw, "enabled", "fee_bps", "recipient_id", "updated_by", "updated_at", "version", "operation_id"); err != nil {
		return err
	}
	*r = LegacyMarketConfigUpdateReceipt{LegacyMarketConfig: LegacyMarketConfig{Enabled: next.Enabled, FeeBPS: next.FeeBPS, RecipientID: next.RecipientID, UpdatedBy: next.UpdatedBy, UpdatedAt: next.UpdatedAt, Version: next.Version}, OperationID: next.OperationID}
	return nil
}

func validLegacyMarketConfig(c LegacyMarketConfig, now int64) bool {
	if c.FeeBPS < 0 || c.FeeBPS > 10000 || c.RecipientID < 0 || c.UpdatedBy < 0 || c.UpdatedAt < 0 || c.UpdatedAt > now || c.Version < 1 {
		return false
	}
	// The explicit fresh Core schema seed is the only unconfigured revision.
	if c.Version == 1 {
		return !c.Enabled && c.FeeBPS == 0 && c.RecipientID == 0 && c.UpdatedBy == 0 && c.UpdatedAt == 0
	}
	return c.RecipientID > 0 && c.UpdatedBy > 0 && c.UpdatedAt > 0
}

func (m *Module) LegacyReadConfig(ctx context.Context) (LegacyMarketConfig, error) {
	if m == nil || m.authority == nil || ctx == nil || ctx.Err() != nil {
		return LegacyMarketConfig{}, ErrUnavailable
	}
	port, ok := m.authority.(CoreLegacyConfig)
	if !ok || missingInterface(port) {
		return LegacyMarketConfig{}, ErrUnavailable
	}
	out, err := port.ReadLegacyConfig(ctx)
	if err != nil {
		return LegacyMarketConfig{}, err
	}
	if !validLegacyMarketConfig(out, m.now().Unix()) {
		return LegacyMarketConfig{}, ErrDenied
	}
	return out, nil
}
func (m *Module) LegacySetConfig(ctx context.Context, credential, account, operation string, in LegacyMarketConfigUpdate) (LegacyMarketConfigUpdateReceipt, error) {
	if !validID.MatchString(operation) || in.OperationID != "" && in.OperationID != operation || in.FeeBPS < 0 || in.FeeBPS > 10000 || in.RecipientID <= 0 {
		return LegacyMarketConfigUpdateReceipt{}, ErrInvalid
	}
	actor, err := m.WebAuthorize(ctx, credential, account, "toolmarket.config")
	if err != nil {
		return LegacyMarketConfigUpdateReceipt{}, err
	}
	port, ok := m.authority.(CoreLegacyConfig)
	if !ok || missingInterface(port) {
		return LegacyMarketConfigUpdateReceipt{}, ErrUnavailable
	}
	before, err := m.LegacyReadConfig(ctx)
	if err != nil {
		return LegacyMarketConfigUpdateReceipt{}, err
	}
	in.OperationID = operation
	out, err := port.UpdateLegacyConfig(ctx, credential, in)
	if err != nil {
		return LegacyMarketConfigUpdateReceipt{}, err
	}
	if out.OperationID != operation || out.Enabled != in.Enabled || out.FeeBPS != in.FeeBPS || out.RecipientID != in.RecipientID || out.UpdatedBy != actor || out.UpdatedAt <= 0 || out.Version < before.Version || out.Version == before.Version && (before.Enabled != in.Enabled || before.FeeBPS != in.FeeBPS || before.RecipientID != in.RecipientID) || !validLegacyMarketConfig(out.LegacyMarketConfig, m.now().Unix()) {
		return LegacyMarketConfigUpdateReceipt{}, ErrDenied
	}
	// Core repeats root/recipient checks inside the durable transaction. No Go
	// config mirror is written, and a stale operation cannot restore an old policy.
	if current, err := m.WebAuthorize(ctx, credential, account, "toolmarket.config"); err != nil {
		return LegacyMarketConfigUpdateReceipt{}, err
	} else if current != actor {
		return LegacyMarketConfigUpdateReceipt{}, ErrDenied
	}
	return out, nil
}
