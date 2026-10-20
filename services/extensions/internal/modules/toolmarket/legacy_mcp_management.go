// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package toolmarket

import (
	"context"
	"sort"
	"strconv"
)

func (m *Module) legacyMCPManagement(ctx context.Context, raw string, p LegacyMarketPrincipal, in legacyMCPInput) (any, error) {
	port, e := m.legacyMarketMetaPort()
	if e != nil {
		return nil, e
	}
	owner := Principal{User: p.UserID, Account: p.AccountID}
	op := digest([]any{"legacy-market-meta-v2", p.UserID, p.ClientID, p.CredentialKind, p.CredentialID, p.AuthVersion, in})
	switch in.Action {
	case "status":
		out, e := port.ReadLegacyMarketMeta(ctx, raw)
		if e != nil {
			return nil, e
		}
		if out.Owner != owner || out.ClientID != p.ClientID || !legacyMetaDelegationValid(out.Delegation, m.now().Unix()) {
			return nil, ErrDenied
		}
		return map[string]any{"free": true, "can_invoke": p.CanInvoke, "can_manage": p.CanManage, "delegation": out.Delegation, "credits_per_usd": 500000, "invoke_supported": true, "invoke_requires_tools_list_refresh": false, "invoke_uses_target_pricing": true, "authorization_help": "Configure AI tool management in this connection's settings. A zero-credit cap permits only free tools. Existing permissions and stricter budgets remain in force."}, nil
	case "authorize":
		if !p.CanInvoke || !p.CanManage {
			return nil, ErrLegacyMarketScope
		}
		request := LegacyMarketMetaAuthorize{OperationID: op, ToolID: in.ToolID, VersionID: in.VersionID, MaxPriceQuota: in.MaxPriceQuota, MaxTotalQuota: in.MaxTotalQuota, MaxCalls: in.MaxCalls, ExpiresAt: in.ExpiresAt}
		out, e := port.AuthorizeLegacyMarketMeta(ctx, raw, request)
		if e != nil {
			return nil, e
		}
		if !legacyMetaReadbackGrant(out, p, m.now().Unix()) || out.ToolID != in.ToolID || out.VersionID != in.VersionID || out.MaxPriceQuota != in.MaxPriceQuota || out.MaxTotalQuota != in.MaxTotalQuota || out.MaxCalls != in.MaxCalls || out.ExpiresAt != in.ExpiresAt || out.RevokedAt != 0 {
			return nil, ErrDenied
		}
		if e = m.legacyMetaSyncGrant(out, p); e != nil {
			return nil, e
		}
		return map[string]any{"grant_id": out.ID, "tool_id": out.ToolID, "version_id": out.VersionID, "max_price_quota": out.MaxPriceQuota, "max_total_quota": out.MaxTotalQuota, "max_calls": out.MaxCalls, "expires_at": out.ExpiresAt, "refresh_tools_list": true}, nil
	case "set_client_budget":
		if !p.CanInvoke || !p.CanManage {
			return nil, ErrLegacyMarketScope
		}
		out, e := port.TightenLegacyMarketClientBudget(ctx, raw, LegacyMarketMetaClientBudget{OperationID: op, LimitQuota: in.LimitQuota})
		if e != nil {
			return nil, e
		}
		if out.Owner != owner || out.OperationID != op || out.Scope != "client" || out.ScopeID != p.ClientID || out.LimitQuota != in.LimitQuota || !legacyReadbackCounters(out.LimitQuota, out.ReservedQuota, out.SpentQuota) {
			return nil, ErrDenied
		}
		return map[string]any{"limit_quota": out.LimitQuota}, nil
	case "set_tool_budget":
		if !p.CanInvoke || !p.CanManage {
			return nil, ErrLegacyMarketScope
		}
		out, e := port.TightenLegacyMarketToolBudget(ctx, raw, LegacyMarketMetaToolBudget{OperationID: op, GrantID: in.GrantID, ToolID: in.ToolID, VersionID: in.VersionID, LimitQuota: in.LimitQuota})
		if e != nil {
			return nil, e
		}
		if !legacyMetaReadbackGrant(out, p, m.now().Unix()) || out.ID != in.GrantID || out.ToolID != in.ToolID || out.VersionID != in.VersionID || out.MaxTotalQuota != in.LimitQuota || out.RevokedAt != 0 {
			return nil, ErrDenied
		}
		if e = m.legacyMetaSyncGrant(out, p); e != nil {
			return nil, e
		}
		return map[string]any{"grant_id": out.ID, "max_total_quota": out.MaxTotalQuota}, nil
	case "usage":
		out, e := port.ReadLegacyMarketUsage(ctx, raw, LegacyMarketMetaPage{Offset: in.Offset, Limit: in.Limit})
		if e != nil {
			return nil, e
		}
		if out.Owner != owner || out.ClientID != p.ClientID || out.Offset != in.Offset || out.Limit != in.Limit || out.Grants == nil || len(out.Grants) > in.Limit {
			return nil, ErrDenied
		}
		var budget any
		if b := out.ClientBudget; b != nil {
			if b.Owner != owner || b.Scope != "client" || b.ScopeID != p.ClientID || !legacyReadbackCounters(b.LimitQuota, b.ReservedQuota, b.SpentQuota) {
				return nil, ErrDenied
			}
			budget = map[string]any{"limit_quota": b.LimitQuota, "reserved_quota": b.ReservedQuota, "spent_quota": b.SpentQuota}
		}
		grants := make([]map[string]any, 0, len(out.Grants))
		seen := map[string]bool{}
		for _, g := range out.Grants {
			if !legacyMetaReadbackGrant(g, p, m.now().Unix()) || seen[g.ID] || g.RevokedAt != 0 || g.ExpiresAt <= m.now().Unix() {
				return nil, ErrDenied
			}
			seen[g.ID] = true
			grants = append(grants, map[string]any{"grant_id": g.ID, "tool_id": g.ToolID, "version_id": g.VersionID, "max_price_quota": g.MaxPriceQuota, "max_total_quota": g.MaxTotalQuota, "max_calls": g.MaxCalls, "reserved_quota": g.ReservedQuota, "spent_quota": g.SpentQuota, "reserved_calls": g.ReservedCalls, "successful_calls": g.SuccessfulCalls, "expires_at": g.ExpiresAt})
		}
		user, _ := strconv.ParseInt(p.UserID, 10, 64)
		var installed []WebInstallation
		e = m.store.view(func(d database) error {
			for _, i := range webState(&d).Installations {
				if i.UserID == user && i.ClientID == p.ClientID {
					installed = append(installed, i)
				}
			}
			return nil
		})
		if e != nil {
			return nil, e
		}
		sort.Slice(installed, func(i, j int) bool { return installed[i].ToolID < installed[j].ToolID })
		installed = webSlice(installed, in.Offset, in.Limit+1)
		more := len(installed) > in.Limit
		if more {
			installed = installed[:in.Limit]
		}
		installs := make([]map[string]string, 0, len(installed))
		for _, i := range installed {
			installs = append(installs, map[string]string{"tool_id": i.ToolID, "version_id": i.VersionID})
		}
		return map[string]any{"client_budget": budget, "installations": installs, "grants": grants, "offset": in.Offset, "limit": in.Limit, "more_installations": more, "more_grants": out.MoreGrants}, nil
	case "calls":
		out, e := port.ReadLegacyMarketCalls(ctx, raw, LegacyMarketMetaPage{Offset: in.Offset, Limit: in.Limit})
		if e != nil {
			return nil, e
		}
		if out.Owner != owner || out.ClientID != p.ClientID || out.Offset != in.Offset || out.Limit != in.Limit || out.Calls == nil || len(out.Calls) > in.Limit {
			return nil, ErrDenied
		}
		seen := map[string]bool{}
		for _, c := range out.Calls {
			if !validID.MatchString(c.CallID) || !validID.MatchString(c.ServiceID) || !validID.MatchString(c.ToolID) || !validID.MatchString(c.VersionID) || seen[c.CallID] || !legacyQuotaValid(c.PriceQuota) || c.CreatedAt <= 0 || c.CreatedAt > m.now().Unix() || c.StartedAt < 0 || c.FinishedAt < 0 || c.StartedAt > m.now().Unix() || c.FinishedAt > m.now().Unix() || c.StartedAt != 0 && c.StartedAt < c.CreatedAt || c.FinishedAt != 0 && (c.FinishedAt < c.CreatedAt || c.FinishedAt < c.StartedAt) {
				return nil, ErrDenied
			}
			seen[c.CallID] = true
			switch c.ExecutionStatus {
			case "reserved":
				if c.StartedAt != 0 || c.FinishedAt != 0 {
					return nil, ErrDenied
				}
			case "running", "unknown":
				if c.StartedAt == 0 || c.FinishedAt != 0 {
					return nil, ErrDenied
				}
			case "succeeded", "failed":
				if c.StartedAt == 0 || c.FinishedAt == 0 {
					return nil, ErrDenied
				}
			case "cancelled":
				if c.FinishedAt == 0 {
					return nil, ErrDenied
				}
			default:
				return nil, ErrDenied
			}
			if c.SettlementStatus != "held" && c.SettlementStatus != "settled" && c.SettlementStatus != "released" {
				return nil, ErrDenied
			}
			if c.ExecutionStatus == "succeeded" && c.SettlementStatus != "settled" || (c.ExecutionStatus == "failed" || c.ExecutionStatus == "cancelled") && c.SettlementStatus != "released" || (c.ExecutionStatus == "reserved" || c.ExecutionStatus == "running" || c.ExecutionStatus == "unknown") && c.SettlementStatus != "held" {
				return nil, ErrDenied
			}
		}
		return map[string]any{"calls": out.Calls, "offset": in.Offset, "limit": in.Limit, "more": out.More}, nil
	default:
		return nil, ErrInvalid
	}
}
