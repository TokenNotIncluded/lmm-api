// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package toolmarket

import (
	"bytes"
	"context"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"strconv"
	"strings"
)

// CoreLegacyAccess is the legacy-quota authority port. It is separate from
// native USD approvals: Core owns policy records, token secrets, authoritative
// held/spent counters, revocation and the reservation transaction.
// All mutations must be idempotent by (owner, operation_id) and immutable input.
// Transport is POST /internal/tool-market/v2/access/<action> with protocol 2.
type CoreLegacyAccess interface {
	ListLegacyConsents(context.Context, string, CoreLegacyListRequest) (CoreLegacyConsentList, error)
	ListLegacyBudgets(context.Context, string, CoreLegacyListRequest) (CoreLegacyBudgetList, error)
	ListLegacyTokens(context.Context, string, CoreLegacyListRequest) (CoreLegacyTokenList, error)
	CreateLegacyConsent(context.Context, string, CoreLegacyConsentRequest) (CoreLegacyConsentReceipt, error)
	RevokeLegacyConsent(context.Context, string, CoreLegacyIDRequest) (CoreLegacyRevocationReceipt, error)
	SetLegacyBudget(context.Context, string, CoreLegacyBudgetRequest) (CoreLegacyBudgetReceipt, error)
	CreateLegacyToken(context.Context, string, CoreLegacyTokenRequest) (CoreLegacyTokenReceipt, error)
	RevokeLegacyToken(context.Context, string, CoreLegacyIDRequest) (CoreLegacyRevocationReceipt, error)
	ValidateLegacyToken(context.Context, string, CoreLegacyValidateRequest) (CoreLegacyTokenReceipt, error)
	DisconnectLegacyClient(context.Context, string, CoreLegacyClientRequest) (CoreLegacyDisconnectReceipt, error)
}
type CoreLegacyOperation struct {
	Owner       Principal `json:"owner"`
	OperationID string    `json:"operation_id"`
}
type CoreLegacyConsentRequest struct {
	CoreLegacyOperation
	ClientID      string `json:"client_id"`
	ToolID        string `json:"tool_id"`
	VersionID     string `json:"version_id"`
	MaxPriceQuota int64  `json:"max_price_quota"`
	MaxTotalQuota int64  `json:"max_total_quota"`
	MaxCalls      int64  `json:"max_calls"`
	ExpiresAt     int64  `json:"expires_at"`
}
type CoreLegacyConsentReceipt struct {
	CoreLegacyConsentRequest
	QuoteID          string `json:"quote_id"`
	FrozenPriceQuota int64  `json:"frozen_price_quota"`
	RulesHash        string `json:"rules_hash"`
	ID               string `json:"id"`
	CreatedAt        int64  `json:"created_at"`
	RevokedAt        int64  `json:"revoked_at"`
	ReservedQuota    int64  `json:"reserved_quota"`
	SpentQuota       int64  `json:"spent_quota"`
	ReservedCalls    int64  `json:"reserved_calls"`
	SuccessfulCalls  int64  `json:"successful_calls"`
}
type CoreLegacyIDRequest struct {
	CoreLegacyOperation
	ID string `json:"id"`
}
type CoreLegacyRevocationReceipt struct {
	CoreLegacyIDRequest
	RevokedAt int64 `json:"revoked_at"`
}
type CoreLegacyBudgetRequest struct {
	CoreLegacyOperation
	Scope      string `json:"scope"`
	ScopeID    string `json:"scope_id"`
	LimitQuota int64  `json:"limit_quota"`
}
type CoreLegacyBudgetReceipt struct {
	CoreLegacyBudgetRequest
	ReservedQuota int64 `json:"reserved_quota"`
	SpentQuota    int64 `json:"spent_quota"`
}
type CoreLegacyTokenRequest struct {
	CoreLegacyOperation
	ClientID  string `json:"client_id"`
	CanInvoke bool   `json:"can_invoke"`
	CanManage bool   `json:"can_manage"`
	ExpiresAt int64  `json:"expires_at"`
}
type CoreLegacyTokenReceipt struct {
	CoreLegacyTokenRequest
	ID          string `json:"id"`
	AuthVersion int64  `json:"auth_version"`
	CreatedAt   int64  `json:"created_at"`
	RevokedAt   int64  `json:"revoked_at"`
	// Raw is returned only by token-create and never persisted in Go.
	Raw      string `json:"token,omitempty"`
	Replayed bool   `json:"replayed"`
}
type CoreLegacyValidateRequest struct {
	Account string `json:"account"`
}
type CoreLegacyClientRequest struct {
	CoreLegacyOperation
	ClientID string `json:"client_id"`
}
type CoreLegacyDisconnectReceipt struct {
	CoreLegacyClientRequest
	TokensRevoked int64 `json:"tokens_revoked"`
	GrantsRevoked int64 `json:"grants_revoked"`
	RevokedAt     int64 `json:"revoked_at"`
}

func (m *Module) coreLegacyAccess(ctx context.Context, credential, account, operation string) (CoreLegacyAccess, CoreLegacyOperation, error) {
	if !validID.MatchString(operation) {
		return nil, CoreLegacyOperation{}, ErrInvalid
	}
	user, err := m.WebAuthorize(ctx, credential, account, "toolmarket.authorize")
	if err != nil {
		return nil, CoreLegacyOperation{}, err
	}
	port, ok := m.authority.(CoreLegacyAccess)
	if !ok || missingInterface(port) {
		return nil, CoreLegacyOperation{}, ErrUnavailable
	}
	return port, CoreLegacyOperation{Owner: Principal{User: strconv.FormatInt(user, 10), Account: account}, OperationID: operation}, nil
}
func legacyReadbackOperation(got, want CoreLegacyOperation) bool {
	return got == want && got.Owner.valid()
}
func legacyReadbackCounters(limit, reserved, spent int64) bool {
	return legacyQuotaValid(limit) && legacyQuotaValid(reserved) && legacyQuotaValid(spent) && spent <= limit && reserved <= limit-spent
}
func legacyGrantFromReceipt(r CoreLegacyConsentReceipt, user int64) LegacyConsentGrant {
	return LegacyConsentGrant{ID: r.ID, UserID: user, ClientID: r.ClientID, ToolID: r.ToolID, VersionID: r.VersionID, MaxPriceQuota: r.MaxPriceQuota, MaxTotalQuota: r.MaxTotalQuota, MaxCalls: r.MaxCalls, CreatedAt: r.CreatedAt, ExpiresAt: r.ExpiresAt, RevokedAt: r.RevokedAt, QuoteID: r.QuoteID, FrozenPriceQuota: r.FrozenPriceQuota, RulesHash: r.RulesHash}
}
func (m *Module) LegacyCreateConsent(ctx context.Context, credential, account, operation string, in LegacyConsentGrant) (CoreLegacyConsentReceipt, error) {
	port, op, err := m.coreLegacyAccess(ctx, credential, account, operation)
	if err != nil {
		return CoreLegacyConsentReceipt{}, err
	}
	now := m.now().Unix()
	user, _ := strconv.ParseInt(op.Owner.User, 10, 64)
	if !legacyClientValid(in.ClientID) || !legacyQuotaValid(in.MaxPriceQuota) || !legacyQuotaValid(in.MaxTotalQuota) || in.MaxCalls < 1 || in.MaxCalls > 1000000 || in.ExpiresAt <= now {
		return CoreLegacyConsentReceipt{}, ErrInvalid
	}
	if err = m.store.view(func(d database) error { _, e := legacyLiveTool(&d, user, in.ToolID, in.VersionID); return e }); err != nil {
		return CoreLegacyConsentReceipt{}, err
	}
	req := CoreLegacyConsentRequest{CoreLegacyOperation: op, ClientID: in.ClientID, ToolID: in.ToolID, VersionID: in.VersionID, MaxPriceQuota: in.MaxPriceQuota, MaxTotalQuota: in.MaxTotalQuota, MaxCalls: in.MaxCalls, ExpiresAt: in.ExpiresAt}
	out, err := port.CreateLegacyConsent(ctx, credential, req)
	if err != nil {
		return CoreLegacyConsentReceipt{}, err
	}
	if out.CoreLegacyConsentRequest != req || !validID.MatchString(out.QuoteID) || !legacyRulesHash(out.RulesHash) || !legacyQuotaValid(out.FrozenPriceQuota) || out.FrozenPriceQuota > req.MaxPriceQuota || !validID.MatchString(out.ID) || out.CreatedAt <= 0 || out.CreatedAt > m.now().Unix() || out.RevokedAt != 0 || !legacyReadbackCounters(out.MaxTotalQuota, out.ReservedQuota, out.SpentQuota) || !legacyReadbackCounters(out.MaxCalls, out.ReservedCalls, out.SuccessfulCalls) {
		return CoreLegacyConsentReceipt{}, ErrDenied
	}
	err = m.store.update(func(d *database) error {
		if err := ctx.Err(); err != nil {
			return err
		}
		if _, e := legacyLiveTool(d, user, in.ToolID, in.VersionID); e != nil {
			return e
		}
		a := legacyAccess(d)
		g := legacyGrantFromReceipt(out, user)
		if _, ok := a.Grants[g.ID]; !ok && len(a.Grants) >= 50000 {
			return ErrLimit
		}
		if prior, ok := a.Grants[g.ID]; ok {
			if prior.UserID != g.UserID || prior.ClientID != g.ClientID || prior.ToolID != g.ToolID || prior.VersionID != g.VersionID || prior.QuoteID != g.QuoteID || prior.FrozenPriceQuota != g.FrozenPriceQuota || prior.RulesHash != g.RulesHash || prior.MaxPriceQuota != g.MaxPriceQuota || prior.MaxTotalQuota != g.MaxTotalQuota || prior.MaxCalls != g.MaxCalls || prior.ExpiresAt != g.ExpiresAt || prior.RevokedAt != 0 {
				return ErrConflict
			}
			g.Hidden = prior.Hidden
		}
		a.Grants[g.ID] = g
		d.audit(op.Owner, "legacy.grant.create", g.ID, "core-confirmed")
		return nil
	})
	return out, err
}
func (m *Module) LegacyRevokeConsent(ctx context.Context, credential, account, operation, id string) error {
	port, op, err := m.coreLegacyAccess(ctx, credential, account, operation)
	if err != nil {
		return err
	}
	user, _ := strconv.ParseInt(op.Owner.User, 10, 64)
	if err = m.store.view(func(d database) error {
		g, ok := legacyAccess(&d).Grants[id]
		if !ok || g.UserID != user {
			return ErrMissing
		}
		return nil
	}); err != nil {
		return err
	}
	req := CoreLegacyIDRequest{CoreLegacyOperation: op, ID: id}
	out, err := port.RevokeLegacyConsent(ctx, credential, req)
	if err != nil {
		return err
	}
	if out.CoreLegacyIDRequest != req || out.RevokedAt <= 0 || out.RevokedAt > m.now().Unix() {
		return ErrDenied
	}
	return m.store.update(func(d *database) error {
		if err := legacyRevokeGrant(d, user, out.RevokedAt, id); err != nil {
			return err
		}
		d.audit(op.Owner, "legacy.grant.revoke", id, "core-confirmed")
		return nil
	})
}
func (m *Module) LegacySetBudget(ctx context.Context, credential, account, operation string, in LegacyBudgetRecord) (CoreLegacyBudgetReceipt, error) {
	port, op, err := m.coreLegacyAccess(ctx, credential, account, operation)
	if err != nil {
		return CoreLegacyBudgetReceipt{}, err
	}
	user, _ := strconv.ParseInt(op.Owner.User, 10, 64)
	preview := emptyDB()
	if err = legacySetBudget(&preview, user, in); err != nil {
		return CoreLegacyBudgetReceipt{}, err
	}
	req := CoreLegacyBudgetRequest{CoreLegacyOperation: op, Scope: in.Scope, ScopeID: in.ScopeID, LimitQuota: in.LimitQuota}
	out, err := port.SetLegacyBudget(ctx, credential, req)
	if err != nil {
		return CoreLegacyBudgetReceipt{}, err
	}
	if out.CoreLegacyBudgetRequest != req || !legacyReadbackCounters(out.LimitQuota, out.ReservedQuota, out.SpentQuota) {
		return CoreLegacyBudgetReceipt{}, ErrDenied
	}
	err = m.store.update(func(d *database) error {
		if err := legacySetBudget(d, user, in); err != nil {
			return err
		}
		d.audit(op.Owner, "legacy.budget.set", in.Scope+":"+in.ScopeID, "core-confirmed")
		return nil
	})
	return out, err
}
func validLegacyPersonalClient(client string) bool {
	return legacyClientValid(client) && client != "web-market" && !strings.HasPrefix(client, "oauth:")
}
func (m *Module) LegacyCreateToken(ctx context.Context, credential, account, operation string, in CoreLegacyTokenRequest) (CoreLegacyTokenReceipt, error) {
	port, op, err := m.coreLegacyAccess(ctx, credential, account, operation)
	if err != nil {
		return CoreLegacyTokenReceipt{}, err
	}
	now := m.now().Unix()
	if !validLegacyPersonalClient(in.ClientID) || in.ExpiresAt <= now || in.ExpiresAt > now+90*86400 {
		return CoreLegacyTokenReceipt{}, ErrInvalid
	}
	in.CoreLegacyOperation = op
	out, err := port.CreateLegacyToken(ctx, credential, in)
	if err != nil {
		return CoreLegacyTokenReceipt{}, err
	}
	if out.CoreLegacyTokenRequest != in || !validID.MatchString(out.ID) || out.CreatedAt <= 0 || out.CreatedAt > m.now().Unix() || out.AuthVersion < 0 || out.RevokedAt != 0 || (!out.Replayed && !legacyRawToken(out.Raw) || out.Replayed && out.Raw != "") {
		return CoreLegacyTokenReceipt{}, ErrDenied
	}
	user, _ := strconv.ParseInt(op.Owner.User, 10, 64)
	err = m.store.update(func(d *database) error {
		if err := legacyRecordToken(d, user, LegacyTokenRecord{ID: out.ID, ClientID: out.ClientID, CreatedAt: out.CreatedAt, ExpiresAt: out.ExpiresAt, CanInvoke: out.CanInvoke, CanManage: out.CanManage}); err != nil {
			return err
		}
		d.audit(op.Owner, "legacy.token.create", out.ID, "core-confirmed")
		return nil
	})
	if err != nil {
		out.Raw = ""
	}
	return out, err
}
func (m *Module) LegacyRevokeToken(ctx context.Context, credential, account, operation, id string) error {
	port, op, err := m.coreLegacyAccess(ctx, credential, account, operation)
	if err != nil {
		return err
	}
	user, _ := strconv.ParseInt(op.Owner.User, 10, 64)
	if err = m.store.view(func(d database) error {
		t, ok := legacyAccess(&d).Tokens[id]
		if !ok || t.UserID != user {
			return ErrMissing
		}
		return nil
	}); err != nil {
		return err
	}
	req := CoreLegacyIDRequest{CoreLegacyOperation: op, ID: id}
	out, err := port.RevokeLegacyToken(ctx, credential, req)
	if err != nil {
		return err
	}
	if out.CoreLegacyIDRequest != req || out.RevokedAt <= 0 || out.RevokedAt > m.now().Unix() {
		return ErrDenied
	}
	return m.store.update(func(d *database) error {
		a := legacyAccess(d)
		t, ok := a.Tokens[id]
		if !ok || t.UserID != user {
			return ErrMissing
		}
		t.RevokedAt = out.RevokedAt
		a.Tokens[id] = t
		d.audit(op.Owner, "legacy.token.revoke", id, "core-confirmed")
		return nil
	})
}
func (m *Module) LegacyValidateClientToken(ctx context.Context, token, account string) (LegacyMetaSubject, error) {
	if m == nil || m.authority == nil || !legacyRawToken(token) {
		return LegacyMetaSubject{}, ErrDenied
	}
	port, ok := m.authority.(CoreLegacyAccess)
	if !ok || missingInterface(port) {
		return LegacyMetaSubject{}, ErrUnavailable
	}
	out, err := port.ValidateLegacyToken(ctx, token, CoreLegacyValidateRequest{Account: account})
	if err != nil {
		return LegacyMetaSubject{}, err
	}
	user, e := strconv.ParseInt(out.Owner.User, 10, 64)
	if e != nil || user <= 0 || out.Owner.Account != account || account != strconv.FormatInt(user, 10) || !out.Owner.valid() || out.OperationID != "" || out.Replayed || out.Raw != "" || !validID.MatchString(out.ID) || !validLegacyPersonalClient(out.ClientID) || out.AuthVersion < 0 || out.CreatedAt <= 0 || out.ExpiresAt <= m.now().Unix() || out.RevokedAt != 0 {
		return LegacyMetaSubject{}, ErrDenied
	}
	return LegacyMetaSubject{UserID: user, ClientID: out.ClientID, CredentialKind: "personal", CredentialID: out.ID, AuthVersion: out.AuthVersion, ExpiresAt: out.ExpiresAt, CanInvoke: out.CanInvoke, CanManage: out.CanManage}, nil
}
func (m *Module) LegacyDisconnectClient(ctx context.Context, credential, account, operation, client string) (LegacyClientChange, error) {
	port, op, err := m.coreLegacyAccess(ctx, credential, account, operation)
	if err != nil {
		return LegacyClientChange{}, err
	}
	if !validLegacyPersonalClient(client) {
		return LegacyClientChange{}, ErrInvalid
	}
	req := CoreLegacyClientRequest{CoreLegacyOperation: op, ClientID: client}
	out, err := port.DisconnectLegacyClient(ctx, credential, req)
	if err != nil {
		return LegacyClientChange{}, err
	}
	if out.CoreLegacyClientRequest != req || out.RevokedAt <= 0 || out.RevokedAt > m.now().Unix() || out.TokensRevoked < 0 || out.GrantsRevoked < 0 {
		return LegacyClientChange{}, ErrDenied
	}
	user, _ := strconv.ParseInt(op.Owner.User, 10, 64)
	var local LegacyClientChange
	err = m.store.update(func(d *database) error {
		var e error
		local, e = legacyChangeClient(d, user, out.RevokedAt, client, false)
		if e != nil {
			return e
		}
		d.audit(op.Owner, "legacy.client.disconnect", digest(client), "core-confirmed")
		return nil
	})
	if err != nil {
		return LegacyClientChange{}, err
	}
	local.TokensRevoked = out.TokensRevoked
	local.GrantsRevoked = out.GrantsRevoked
	return local, nil
}

func legacyRulesHash(s string) bool {
	b, e := hex.DecodeString(s)
	return e == nil && len(b) == 32 && s == strings.ToLower(s)
}
func legacyRawToken(s string) bool {
	if !strings.HasPrefix(s, "lmm_market_") || len(s) != 54 {
		return false
	}
	b, e := base64.RawURLEncoding.DecodeString(strings.TrimPrefix(s, "lmm_market_"))
	return e == nil && len(b) == 32
}

// List operations are live owner-scoped reads; counters always come from Core.
type CoreLegacyListRequest struct {
	Owner  Principal `json:"owner"`
	Offset int       `json:"offset"`
	Limit  int       `json:"limit"`
}
type CoreLegacyConsentList struct {
	Owner Principal                  `json:"owner"`
	Rows  []CoreLegacyConsentReceipt `json:"rows"`
}
type CoreLegacyBudgetList struct {
	Owner Principal                 `json:"owner"`
	Rows  []CoreLegacyBudgetReceipt `json:"rows"`
}
type CoreLegacyTokenList struct {
	Owner Principal                `json:"owner"`
	Rows  []CoreLegacyTokenReceipt `json:"rows"`
}

func (m *Module) coreLegacyList(ctx context.Context, credential, account string, offset, limit int) (CoreLegacyAccess, CoreLegacyListRequest, error) {
	if !validWebPage(offset, limit) {
		return nil, CoreLegacyListRequest{}, ErrInvalid
	}
	user, e := m.WebAuthorize(ctx, credential, account, "toolmarket.read")
	if e != nil {
		return nil, CoreLegacyListRequest{}, e
	}
	port, ok := m.authority.(CoreLegacyAccess)
	if !ok || missingInterface(port) {
		return nil, CoreLegacyListRequest{}, ErrUnavailable
	}
	return port, CoreLegacyListRequest{Owner: Principal{User: strconv.FormatInt(user, 10), Account: account}, Offset: offset, Limit: limit}, nil
}
func (m *Module) LegacyListConsents(ctx context.Context, credential, account string, offset, limit int) (CoreLegacyConsentList, error) {
	port, req, e := m.coreLegacyList(ctx, credential, account, offset, limit)
	if e != nil {
		return CoreLegacyConsentList{}, e
	}
	out, e := port.ListLegacyConsents(ctx, credential, req)
	if e != nil {
		return CoreLegacyConsentList{}, e
	}
	if out.Owner != req.Owner || out.Rows == nil || len(out.Rows) > limit {
		return CoreLegacyConsentList{}, ErrDenied
	}
	seen := map[string]bool{}
	for _, r := range out.Rows {
		if r.Owner != req.Owner || !validID.MatchString(r.ID) || seen[r.ID] || !legacyClientValid(r.ClientID) || !validID.MatchString(r.ToolID) || !validID.MatchString(r.VersionID) || !validID.MatchString(r.QuoteID) || !legacyRulesHash(r.RulesHash) || !legacyQuotaValid(r.FrozenPriceQuota) || !legacyQuotaValid(r.MaxPriceQuota) || r.FrozenPriceQuota > r.MaxPriceQuota || !legacyReadbackCounters(r.MaxTotalQuota, r.ReservedQuota, r.SpentQuota) || r.MaxCalls < 1 || r.MaxCalls > 1000000 || !legacyReadbackCounters(r.MaxCalls, r.ReservedCalls, r.SuccessfulCalls) || r.CreatedAt <= 0 || r.ExpiresAt <= r.CreatedAt || r.RevokedAt < 0 {
			return CoreLegacyConsentList{}, ErrDenied
		}
		seen[r.ID] = true
	}
	user, _ := strconv.ParseInt(req.Owner.User, 10, 64)
	if e = m.store.update(func(d *database) error { return legacyMirrorCoreConsents(d, user, out.Rows) }); e != nil {
		return CoreLegacyConsentList{}, e
	}
	return out, nil
}
func (m *Module) LegacyListBudgets(ctx context.Context, credential, account string, offset, limit int) (CoreLegacyBudgetList, error) {
	port, req, e := m.coreLegacyList(ctx, credential, account, offset, limit)
	if e != nil {
		return CoreLegacyBudgetList{}, e
	}
	out, e := port.ListLegacyBudgets(ctx, credential, req)
	if e != nil {
		return CoreLegacyBudgetList{}, e
	}
	if out.Owner != req.Owner || out.Rows == nil || len(out.Rows) > limit {
		return CoreLegacyBudgetList{}, ErrDenied
	}
	seen := map[string]bool{}
	user, _ := strconv.ParseInt(req.Owner.User, 10, 64)
	for _, r := range out.Rows {
		key := r.Scope + ":" + r.ScopeID
		if r.Owner != req.Owner || seen[key] || !legacyReadbackCounters(r.LimitQuota, r.ReservedQuota, r.SpentQuota) {
			return CoreLegacyBudgetList{}, ErrDenied
		}
		scratch := emptyDB()
		if e := legacySetBudget(&scratch, user, LegacyBudgetRecord{Scope: r.Scope, ScopeID: r.ScopeID, LimitQuota: r.LimitQuota}); e != nil {
			return CoreLegacyBudgetList{}, ErrDenied
		}
		seen[key] = true
	}
	return out, nil
}
func (m *Module) LegacyListTokens(ctx context.Context, credential, account string, offset, limit int) (CoreLegacyTokenList, error) {
	port, req, e := m.coreLegacyList(ctx, credential, account, offset, limit)
	if e != nil {
		return CoreLegacyTokenList{}, e
	}
	out, e := port.ListLegacyTokens(ctx, credential, req)
	if e != nil {
		return CoreLegacyTokenList{}, e
	}
	if out.Owner != req.Owner || out.Rows == nil || len(out.Rows) > limit {
		return CoreLegacyTokenList{}, ErrDenied
	}
	seen := map[string]bool{}
	for _, r := range out.Rows {
		if r.Owner != req.Owner || !validID.MatchString(r.ID) || seen[r.ID] || !validLegacyPersonalClient(r.ClientID) || r.AuthVersion < 0 || r.CreatedAt <= 0 || r.ExpiresAt <= r.CreatedAt || r.RevokedAt < 0 || r.Raw != "" || r.Replayed {
			return CoreLegacyTokenList{}, ErrDenied
		}
		seen[r.ID] = true
	}
	user, _ := strconv.ParseInt(req.Owner.User, 10, 64)
	if e = m.store.update(func(d *database) error { return legacyMirrorCoreTokens(d, user, out.Rows) }); e != nil {
		return CoreLegacyTokenList{}, e
	}
	return out, nil
}

// Core lists may contain migrated records not yet mirrored by Go. Keep local
// visibility metadata while refreshing only already-validated authority records.
// LegacyListConsents/LegacyListTokens invoke these in one Store.update.
func legacyMirrorCoreConsents(d *database, user int64, rows []CoreLegacyConsentReceipt) error {
	a := legacyAccess(d)
	for _, r := range rows {
		if r.Owner.User != strconv.FormatInt(user, 10) || r.Owner.Account != r.Owner.User {
			return ErrDenied
		}
		g := legacyGrantFromReceipt(r, user)
		if old, ok := a.Grants[g.ID]; ok {
			if old.UserID != user || old.ClientID != g.ClientID || old.ToolID != g.ToolID || old.VersionID != g.VersionID || old.CreatedAt != g.CreatedAt || old.QuoteID != "" && (old.QuoteID != g.QuoteID || old.FrozenPriceQuota != g.FrozenPriceQuota || old.RulesHash != g.RulesHash) || old.RevokedAt != 0 && g.RevokedAt == 0 {
				return ErrConflict
			}
			g.Hidden = old.Hidden
		} else if len(a.Grants) >= 50000 {
			return ErrLimit
		}
		a.Grants[g.ID] = g
	}
	return nil
}
func legacyMirrorCoreTokens(d *database, user int64, rows []CoreLegacyTokenReceipt) error {
	for _, r := range rows {
		if r.Owner.User != strconv.FormatInt(user, 10) || r.Owner.Account != r.Owner.User || r.Raw != "" || r.Replayed {
			return ErrDenied
		}
		if err := legacyRecordToken(d, user, LegacyTokenRecord{ID: r.ID, ClientID: r.ClientID, CreatedAt: r.CreatedAt, ExpiresAt: r.ExpiresAt, RevokedAt: r.RevokedAt, CanInvoke: r.CanInvoke, CanManage: r.CanManage}); err != nil {
			return err
		}
	}
	return nil
}

// Required zero-valued counters must be present on the wire. Missing or null
// authority counters are unknown, never evidence of zero held/spent funds.
func legacyRequiredFields(raw []byte, keys ...string) error {
	var fields map[string]json.RawMessage
	if e := json.Unmarshal(raw, &fields); e != nil || fields == nil {
		return ErrInvalid
	}
	for _, key := range keys {
		value, ok := fields[key]
		if !ok || bytes.Equal(bytes.TrimSpace(value), []byte("null")) {
			return ErrInvalid
		}
	}
	return nil
}
func (r *CoreLegacyConsentReceipt) UnmarshalJSON(raw []byte) error {
	type alias CoreLegacyConsentReceipt
	var next alias
	if e := decodeStrict(raw, &next); e != nil {
		return e
	}
	if e := legacyRequiredFields(raw, "owner", "operation_id", "client_id", "tool_id", "version_id", "max_price_quota", "max_total_quota", "max_calls", "expires_at", "id", "quote_id", "frozen_price_quota", "rules_hash", "created_at", "revoked_at", "reserved_quota", "spent_quota", "reserved_calls", "successful_calls"); e != nil {
		return e
	}
	*r = CoreLegacyConsentReceipt(next)
	return nil
}
func (r *CoreLegacyBudgetReceipt) UnmarshalJSON(raw []byte) error {
	type alias CoreLegacyBudgetReceipt
	var next alias
	if e := decodeStrict(raw, &next); e != nil {
		return e
	}
	if e := legacyRequiredFields(raw, "owner", "operation_id", "scope", "scope_id", "limit_quota", "reserved_quota", "spent_quota"); e != nil {
		return e
	}
	*r = CoreLegacyBudgetReceipt(next)
	return nil
}
func (r *CoreLegacyTokenReceipt) UnmarshalJSON(raw []byte) error {
	type alias CoreLegacyTokenReceipt
	var next alias
	if e := decodeStrict(raw, &next); e != nil {
		return e
	}
	if e := legacyRequiredFields(raw, "owner", "operation_id", "client_id", "can_invoke", "can_manage", "expires_at", "id", "auth_version", "created_at", "revoked_at", "replayed"); e != nil {
		return e
	}
	*r = CoreLegacyTokenReceipt(next)
	return nil
}

func (r *CoreLegacyRevocationReceipt) UnmarshalJSON(raw []byte) error {
	type alias CoreLegacyRevocationReceipt
	var next alias
	if e := decodeStrict(raw, &next); e != nil {
		return e
	}
	if e := legacyRequiredFields(raw, "owner", "operation_id", "id", "revoked_at"); e != nil {
		return e
	}
	*r = CoreLegacyRevocationReceipt(next)
	return nil
}
func (r *CoreLegacyDisconnectReceipt) UnmarshalJSON(raw []byte) error {
	type alias CoreLegacyDisconnectReceipt
	var next alias
	if e := decodeStrict(raw, &next); e != nil {
		return e
	}
	if e := legacyRequiredFields(raw, "owner", "operation_id", "client_id", "tokens_revoked", "grants_revoked", "revoked_at"); e != nil {
		return e
	}
	*r = CoreLegacyDisconnectReceipt(next)
	return nil
}
