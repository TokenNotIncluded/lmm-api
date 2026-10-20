// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package toolmarket

import (
	"context"
	"encoding/json"
	"errors"
	"strconv"

	"github.com/modelcontextprotocol/go-sdk/mcp"
)

// Core must scope this read to the live credential's client in addition to the
// owner echo. Go rejects any row outside that client before persisting mirrors.
func (m *Module) legacyMCPGrants(ctx context.Context, raw string, p LegacyMarketPrincipal) ([]CoreLegacyConsentReceipt, error) {
	port, ok := m.authority.(CoreLegacyAccess)
	if !ok || missingInterface(port) {
		return nil, ErrUnavailable
	}
	owner := Principal{User: p.UserID, Account: p.AccountID}
	rows := []CoreLegacyConsentReceipt{}
	seen := map[string]bool{}
	for offset := 0; offset <= 10000; offset += 100 {
		page, e := port.ListLegacyConsents(ctx, raw, CoreLegacyListRequest{Owner: owner, Offset: offset, Limit: 100})
		if e != nil {
			return nil, e
		}
		if page.Owner != owner || page.Rows == nil || len(page.Rows) > 100 {
			return nil, ErrDenied
		}
		for _, r := range page.Rows {
			if !legacyMetaReadbackGrant(r, p, m.now().Unix()) || seen[r.ID] {
				return nil, ErrDenied
			}
			seen[r.ID] = true
			rows = append(rows, r)
		}
		if len(page.Rows) < 100 {
			user, _ := strconv.ParseInt(p.UserID, 10, 64)
			e = m.store.update(func(d *database) error { return legacyMirrorCoreConsents(d, user, rows) })
			return rows, e
		}
	}
	return nil, ErrLimit
}
func legacyMCPGrant(rows []CoreLegacyConsentReceipt, tool, version string, now int64) (CoreLegacyConsentReceipt, bool) {
	var best CoreLegacyConsentReceipt
	for _, g := range rows {
		if g.ToolID == tool && g.VersionID == version && g.RevokedAt == 0 && g.ExpiresAt > now && (best.ID == "" || g.CreatedAt > best.CreatedAt || g.CreatedAt == best.CreatedAt && g.ID > best.ID) {
			best = g
		}
	}
	return best, best.ID != ""
}
func (m *Module) legacyMCPInvoke(ctx context.Context, raw string, p LegacyMarketPrincipal, in legacyMCPInput, req *mcp.CallToolRequest) (*mcp.CallToolResult, error) {
	if !p.CanInvoke {
		return legacyMCPOutput(nil, ErrLegacyMarketScope)
	}
	// Recover the original grant across direct and metamcp entry points. Looking
	// up the existing operation cannot dispatch a new call.
	prior, e := m.LegacyGetExecutionByRequestKey(ctx, raw, p.AccountID, p.ClientID, in.RequestID)
	grant := ""
	if e == nil || errors.Is(e, ErrPending) {
		if prior.Input.ClientID != p.ClientID || prior.Input.ToolID != in.ToolID || prior.Input.VersionID != in.VersionID {
			return legacyMCPOutput(nil, ErrConflict)
		}
		grant = prior.Input.GrantID
	} else if !errors.Is(e, ErrMissing) {
		return legacyMCPOutput(nil, e)
	}
	if grant == "" {
		rows, e := m.legacyMCPGrants(ctx, raw, p)
		if e != nil {
			return legacyMCPOutput(nil, e)
		}
		g, ok := legacyMCPGrant(rows, in.ToolID, in.VersionID, m.now().Unix())
		if !ok {
			return legacyMCPOutput(nil, ErrDenied)
		}
		grant = g.ID
	}
	input := LegacyExecuteInput{ClientID: p.ClientID, RequestKey: in.RequestID, ToolID: in.ToolID, VersionID: in.VersionID, GrantID: grant, Arguments: in.Arguments}
	// Outer confirmation state is handled by the existing execution owner. It
	// must never become a business argument or change the original request hash.
	var responses json.RawMessage
	if len(req.Params.InputResponses) > 0 {
		var encodeErr error
		responses, encodeErr = json.Marshal(req.Params.InputResponses)
		if encodeErr != nil {
			return legacyMCPOutput(nil, ErrInvalid)
		}
	}
	r, e := m.LegacyExecuteMCP(ctx, raw, p.AccountID, input, req.Params.RequestState, responses)
	if e != nil && !errors.Is(e, ErrPending) {
		return legacyMCPOutput(nil, e)
	}
	response, e := m.legacyWebExecution(r, errors.Is(e, ErrPending))
	if e != nil {
		return legacyMCPOutput(nil, e)
	}

	if len(response.Result) == 0 {
		response.Result = LegacyBuiltinPublicPreview(r)
	}
	result, e := legacyMCPExecutionOutput(response)
	if e == nil && result != nil && r.Builtin != nil {
		result.RequestState = LegacyMCPRequestState(r)
	}
	return result, e
}
func (m *Module) legacyMCPMeta(ctx context.Context, raw string, p LegacyMarketPrincipal, in legacyMCPInput) (any, error) {
	switch in.Action {
	case "search":
		rows, e := m.PublicWebCatalogue(ctx, raw, p.AccountID, WebQuery{Text: in.Query, Offset: in.Offset, Limit: in.Limit})
		if e != nil {
			return nil, e
		}
		items := make([]map[string]any, 0, len(rows))
		for _, r := range rows {
			desc := []rune(r.Description)
			truncated := len(desc) > 240
			if truncated {
				desc = desc[:240]
			}
			items = append(items, map[string]any{"id": r.ID, "version_id": r.VersionID, "name": r.Name, "description": string(desc), "description_truncated": truncated, "tool_count": r.ToolCount, "min_price_quota": r.MinPriceQuota, "max_price_quota": r.MaxPriceQuota, "metered_tools": r.MeteredTools, "provider_tools": r.ProviderTools})
		}
		return map[string]any{"items": items, "offset": in.Offset, "limit": in.Limit, "credits_per_usd": 500000, "details_action": "details", "pricing_note": "Read details for exact tool schemas and metered rates. Free invocation can still trigger paid business actions."}, nil
	case "details":
		return m.PublicWebDetail(ctx, raw, p.AccountID, in.ServiceID)
	case "call_status":
		// Check the immutable client before querying or returning any Core call.
		e := m.store.view(func(d database) error {
			r, ok := d.LegacyExecutions[in.CallID]
			if !ok || r.Owner.User != p.UserID || r.Owner.Account != p.AccountID || r.Input.ClientID != p.ClientID {
				return ErrMissing
			}
			return nil
		})
		if e != nil {
			return nil, e
		}
		r, e := m.LegacyGetExecution(ctx, raw, p.AccountID, in.CallID)
		if e != nil && !errors.Is(e, ErrPending) {
			return nil, e
		}
		return m.legacyWebExecution(r, errors.Is(e, ErrPending))
	case "load", "unload":
		if !p.CanManage {
			return nil, ErrLegacyMarketScope
		}
		user, _ := strconv.ParseInt(p.UserID, 10, 64)
		body, _ := json.Marshal(map[string]any{"client_id": p.ClientID, "tool_id": in.ToolID, "version_id": in.VersionID, "loaded": in.Action == "load"})
		e := m.webInstall(Principal{User: p.UserID, Account: p.AccountID}, user, body)
		return map[string]any{"loaded": in.Action == "load", "authorization_required": true, "refresh_tools_list": true}, e
	case "status", "usage", "calls", "authorize", "set_tool_budget", "set_client_budget":
		return m.legacyMCPManagement(ctx, raw, p, in)
	default:
		return nil, ErrInvalid
	}
}
