// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package toolmarket

import (
	"context"
)

func (m *Module) legacyWebAccess(ctx context.Context, credential, account string, p Principal, user int64, in WebRequest) (any, error) {
	switch in.Action {
	case "grant", "budget", "grant.revoke", "token", "token.revoke", "client.disconnect":
		if port, ok := m.authority.(CoreLegacyAccess); !ok || missingInterface(port) {
			return nil, ErrFunds
		}
	}
	operation := in.OperationID
	if operation == "" {
		operation = digest([]any{"legacy-toolmarket-web-v2", p, in.Action, in.ID, string(in.Body)})
	}
	switch in.Action {
	case "grant":
		value, err := payload[struct {
			ClientID      string `json:"client_id"`
			ToolID        string `json:"tool_id"`
			VersionID     string `json:"version_id"`
			MaxPriceQuota int64  `json:"max_price_quota"`
			MaxTotalQuota int64  `json:"max_total_quota"`
			MaxCalls      int64  `json:"max_calls"`
			ExpiresAt     int64  `json:"expires_at"`
		}](in.Body, "client_id", "tool_id", "version_id", "max_price_quota", "max_total_quota", "max_calls", "expires_at")
		if err != nil {
			return nil, err
		}
		receipt, err := m.LegacyCreateConsent(ctx, credential, account, operation, LegacyConsentGrant{ClientID: value.ClientID, ToolID: value.ToolID, VersionID: value.VersionID, MaxPriceQuota: value.MaxPriceQuota, MaxTotalQuota: value.MaxTotalQuota, MaxCalls: value.MaxCalls, ExpiresAt: value.ExpiresAt})
		if err != nil {
			return nil, err
		}
		grant := legacyGrantFromReceipt(receipt, user)
		return grant, nil
	case "grant.revoke":
		return nil, m.LegacyRevokeConsent(ctx, credential, account, operation, in.ID)
	case "token":
		value, err := payload[struct {
			ClientID  string `json:"client_id"`
			CanInvoke bool   `json:"can_invoke"`
			CanManage bool   `json:"can_manage"`
			ExpiresAt int64  `json:"expires_at"`
		}](in.Body, "client_id", "expires_at")
		if err != nil {
			return nil, err
		}
		receipt, err := m.LegacyCreateToken(ctx, credential, account, operation, CoreLegacyTokenRequest{ClientID: value.ClientID, CanInvoke: value.CanInvoke, CanManage: value.CanManage, ExpiresAt: value.ExpiresAt})
		if err != nil {
			return nil, err
		}
		record := struct {
			ID        string `json:"id"`
			ClientID  string `json:"client_id"`
			CanInvoke bool   `json:"can_invoke"`
			CanManage bool   `json:"can_manage"`
			CreatedAt int64  `json:"created_at"`
			ExpiresAt int64  `json:"expires_at"`
			RevokedAt int64  `json:"revoked_at"`
		}{receipt.ID, receipt.ClientID, receipt.CanInvoke, receipt.CanManage, receipt.CreatedAt, receipt.ExpiresAt, receipt.RevokedAt}
		return map[string]any{"token": receipt.Raw, "record": record}, nil
	case "token.revoke":
		return nil, m.LegacyRevokeToken(ctx, credential, account, operation, in.ID)
	case "budget":
		value, err := payload[LegacyBudgetRecord](in.Body, "scope", "scope_id", "limit_quota")
		if err != nil {
			return nil, err
		}
		_, err = m.LegacySetBudget(ctx, credential, account, operation, value)
		return nil, err
	case "client.disconnect", "client.remove":
		value, err := payload[struct {
			ClientID string `json:"client_id"`
		}](in.Body, "client_id")
		if err != nil {
			return nil, err
		}
		if in.Action == "client.disconnect" {
			return m.LegacyDisconnectClient(ctx, credential, account, operation, value.ClientID)
		}
		var result LegacyClientChange
		err = m.store.update(func(d *database) error {
			var err error
			result, err = legacyChangeClient(d, user, m.now().Unix(), value.ClientID, true)
			if err == nil {
				d.audit(p, "legacy.client.remove", value.ClientID, "ok")
			}
			return err
		})
		return result, err
	case "grant.hide", "token.hide":
		kind := "grant"
		if in.Action == "token.hide" {
			kind = "token"
		}
		return nil, m.store.update(func(d *database) error {
			if err := legacyHideRecord(d, user, in.ID, kind); err != nil {
				return err
			}
			d.audit(p, "legacy.record.hide", in.ID, "ok")
			return nil
		})
	default:
		return nil, ErrMissing
	}
}
