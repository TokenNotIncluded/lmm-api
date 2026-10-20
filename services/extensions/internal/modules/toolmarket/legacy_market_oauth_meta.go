// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package toolmarket

import (
	"context"
	"encoding/json"
	"strconv"
	"strings"
)

type LegacyMarketDelegation struct {
	Enabled       bool  `json:"enabled"`
	MaxTotalQuota int64 `json:"max_total_quota"`
	ExpiresAt     int64 `json:"expires_at"`
	UpdatedAt     int64 `json:"updated_at"`
}
type LegacyMarketMetaView struct {
	Owner      Principal              `json:"owner"`
	ClientID   string                 `json:"client_id"`
	Delegation LegacyMarketDelegation `json:"delegation"`
}
type LegacyMarketMetaTarget struct {
	Kind string `json:"kind"`
	ID   string `json:"id"`
}
type LegacyMarketMetaSet struct {
	OperationID   string `json:"operation_id"`
	Kind          string `json:"kind"`
	ID            string `json:"id"`
	Enabled       bool   `json:"enabled"`
	MaxTotalQuota int64  `json:"max_total_quota"`
	ExpiresAt     int64  `json:"expires_at"`
}
type LegacyMarketMetaOwnerView struct {
	OperationID string                 `json:"operation_id"`
	Owner       Principal              `json:"owner"`
	Kind        string                 `json:"kind"`
	ID          string                 `json:"id"`
	Delegation  LegacyMarketDelegation `json:"delegation"`
}
type LegacyMarketMetaAuthorize struct {
	OperationID   string `json:"operation_id"`
	ToolID        string `json:"tool_id"`
	VersionID     string `json:"version_id"`
	MaxPriceQuota int64  `json:"max_price_quota"`
	MaxTotalQuota int64  `json:"max_total_quota"`
	MaxCalls      int64  `json:"max_calls"`
	ExpiresAt     int64  `json:"expires_at"`
}
type LegacyMarketMetaClientBudget struct {
	OperationID string `json:"operation_id"`
	LimitQuota  int64  `json:"limit_quota"`
}
type LegacyMarketMetaToolBudget struct {
	OperationID string `json:"operation_id"`
	GrantID     string `json:"grant_id"`
	ToolID      string `json:"tool_id"`
	VersionID   string `json:"version_id"`
	LimitQuota  int64  `json:"limit_quota"`
}
type LegacyMarketMetaPage struct {
	Offset int `json:"offset"`
	Limit  int `json:"limit"`
}
type LegacyMarketMetaUsage struct {
	Owner        Principal                  `json:"owner"`
	ClientID     string                     `json:"client_id"`
	ClientBudget *CoreLegacyBudgetReceipt   `json:"client_budget"`
	Grants       []CoreLegacyConsentReceipt `json:"grants"`
	Offset       int                        `json:"offset"`
	Limit        int                        `json:"limit"`
	MoreGrants   bool                       `json:"more_grants"`
}
type LegacyMarketMetaCall struct {
	CallID           string `json:"call_id"`
	ServiceID        string `json:"service_id"`
	ToolID           string `json:"tool_id"`
	VersionID        string `json:"version_id"`
	PriceQuota       int64  `json:"price_quota"`
	ExecutionStatus  string `json:"execution_status"`
	SettlementStatus string `json:"settlement_status"`
	CreatedAt        int64  `json:"created_at"`
	StartedAt        int64  `json:"started_at"`
	FinishedAt       int64  `json:"finished_at"`
}
type LegacyMarketMetaCalls struct {
	Owner    Principal              `json:"owner"`
	ClientID string                 `json:"client_id"`
	Calls    []LegacyMarketMetaCall `json:"calls"`
	Offset   int                    `json:"offset"`
	Limit    int                    `json:"limit"`
	More     bool                   `json:"more"`
}

type CoreLegacyMarketMeta interface {
	ReadLegacyMarketMeta(context.Context, string) (LegacyMarketMetaView, error)
	ReadLegacyMarketMetaOwner(context.Context, string, LegacyMarketMetaTarget) (LegacyMarketMetaOwnerView, error)
	SetLegacyMarketMetaOwner(context.Context, string, LegacyMarketMetaSet) (LegacyMarketMetaOwnerView, error)
	AuthorizeLegacyMarketMeta(context.Context, string, LegacyMarketMetaAuthorize) (CoreLegacyConsentReceipt, error)
	TightenLegacyMarketClientBudget(context.Context, string, LegacyMarketMetaClientBudget) (CoreLegacyBudgetReceipt, error)
	TightenLegacyMarketToolBudget(context.Context, string, LegacyMarketMetaToolBudget) (CoreLegacyConsentReceipt, error)
	ReadLegacyMarketUsage(context.Context, string, LegacyMarketMetaPage) (LegacyMarketMetaUsage, error)
	ReadLegacyMarketCalls(context.Context, string, LegacyMarketMetaPage) (LegacyMarketMetaCalls, error)
}

func (m *Module) legacyMarketMetaPort() (CoreLegacyMarketMeta, error) {
	p, ok := m.authority.(CoreLegacyMarketMeta)
	if !ok || missingInterface(p) {
		return nil, ErrUnavailable
	}
	return p, nil
}
func legacyMetaDelegationValid(p LegacyMarketDelegation, now int64) bool {
	return legacyQuotaValid(p.MaxTotalQuota) && p.UpdatedAt >= 0 && p.UpdatedAt <= now && (p.Enabled && p.ExpiresAt > now || !p.Enabled && p.ExpiresAt == 0 && p.MaxTotalQuota == 0)
}
func legacyMetaOwnerMatches(out LegacyMarketMetaOwnerView, user int64, account, kind, id, op string) bool {
	return out.Owner == (Principal{User: strconv.FormatInt(user, 10), Account: account}) && out.Kind == kind && out.ID == id && out.OperationID == op
}
func (m *Module) LegacyGetMetaDelegation(ctx context.Context, credential, account, kind, id string) (LegacyMarketDelegation, error) {
	user, e := m.WebAuthorize(ctx, credential, account, "toolmarket.authorize")
	if e != nil {
		return LegacyMarketDelegation{}, e
	}
	if kind != "personal" && kind != "oauth" || id == "" || len(id) > 128 {
		return LegacyMarketDelegation{}, ErrInvalid
	}
	port, e := m.legacyMarketMetaPort()
	if e != nil {
		return LegacyMarketDelegation{}, e
	}
	out, e := port.ReadLegacyMarketMetaOwner(ctx, credential, LegacyMarketMetaTarget{Kind: kind, ID: id})
	if e != nil {
		return LegacyMarketDelegation{}, e
	}
	if !legacyMetaOwnerMatches(out, user, account, kind, id, "") || !legacyMetaDelegationValid(out.Delegation, m.now().Unix()) {
		return LegacyMarketDelegation{}, ErrDenied
	}
	return out.Delegation, nil
}
func (m *Module) LegacySetMetaDelegation(ctx context.Context, credential, account string, in LegacyMarketMetaSet) (LegacyMarketDelegation, error) {
	user, e := m.WebAuthorize(ctx, credential, account, "toolmarket.authorize")
	if e != nil {
		return LegacyMarketDelegation{}, e
	}
	if !validID.MatchString(in.OperationID) || in.Kind != "personal" && in.Kind != "oauth" || in.ID == "" || len(in.ID) > 128 || !legacyQuotaValid(in.MaxTotalQuota) || in.ExpiresAt < 0 || !in.Enabled && (in.MaxTotalQuota != 0 || in.ExpiresAt != 0) {
		return LegacyMarketDelegation{}, ErrInvalid
	}
	port, e := m.legacyMarketMetaPort()
	if e != nil {
		return LegacyMarketDelegation{}, e
	}
	out, e := port.SetLegacyMarketMetaOwner(ctx, credential, in)
	if e != nil {
		return LegacyMarketDelegation{}, e
	}
	if !legacyMetaOwnerMatches(out, user, account, in.Kind, in.ID, in.OperationID) || !legacyMetaDelegationValid(out.Delegation, m.now().Unix()) || out.Delegation.Enabled != in.Enabled || out.Delegation.MaxTotalQuota > in.MaxTotalQuota || in.ExpiresAt != 0 && out.Delegation.ExpiresAt > in.ExpiresAt {
		return LegacyMarketDelegation{}, ErrDenied
	}
	return out.Delegation, nil
}
func legacyMetaReadbackGrant(r CoreLegacyConsentReceipt, p LegacyMarketPrincipal, now int64) bool {
	return r.Owner == (Principal{User: p.UserID, Account: p.AccountID}) && r.ClientID == p.ClientID && validID.MatchString(r.ID) && validID.MatchString(r.ToolID) && validID.MatchString(r.VersionID) && validID.MatchString(r.QuoteID) && legacyRulesHash(r.RulesHash) && legacyQuotaValid(r.FrozenPriceQuota) && legacyQuotaValid(r.MaxPriceQuota) && r.FrozenPriceQuota <= r.MaxPriceQuota && legacyReadbackCounters(r.MaxTotalQuota, r.ReservedQuota, r.SpentQuota) && r.MaxCalls >= 1 && r.MaxCalls <= 1000000 && legacyReadbackCounters(r.MaxCalls, r.ReservedCalls, r.SuccessfulCalls) && r.CreatedAt > 0 && r.CreatedAt <= now && r.ExpiresAt > r.CreatedAt && r.RevokedAt >= 0 && r.RevokedAt <= now
}
func (m *Module) legacyMetaSyncGrant(r CoreLegacyConsentReceipt, p LegacyMarketPrincipal) error {
	if !legacyMetaReadbackGrant(r, p, m.now().Unix()) {
		return ErrDenied
	}
	user, _ := strconv.ParseInt(p.UserID, 10, 64)
	return m.store.update(func(d *database) error { return legacyMirrorCoreConsents(d, user, []CoreLegacyConsentReceipt{r}) })
}

// Receipt zero values must be explicit; omitted permission/counter fields are
// unknown authority rather than invented disabled/zero state.
func (r *LegacyMarketDelegation) UnmarshalJSON(raw []byte) error {
	type alias LegacyMarketDelegation
	var v alias
	if e := decodeStrict(raw, &v); e != nil {
		return e
	}
	if e := legacyRequiredFields(raw, "enabled", "max_total_quota", "expires_at", "updated_at"); e != nil {
		return e
	}
	*r = LegacyMarketDelegation(v)
	return nil
}
func (r *LegacyMarketMetaView) UnmarshalJSON(raw []byte) error {
	type alias LegacyMarketMetaView
	var v alias
	if e := decodeStrict(raw, &v); e != nil {
		return e
	}
	if e := legacyRequiredFields(raw, "owner", "client_id", "delegation"); e != nil {
		return e
	}
	*r = LegacyMarketMetaView(v)
	return nil
}
func (r *LegacyMarketMetaOwnerView) UnmarshalJSON(raw []byte) error {
	type alias LegacyMarketMetaOwnerView
	var v alias
	if e := decodeStrict(raw, &v); e != nil {
		return e
	}
	if e := legacyRequiredFields(raw, "operation_id", "owner", "kind", "id", "delegation"); e != nil {
		return e
	}
	*r = LegacyMarketMetaOwnerView(v)
	return nil
}
func (r *LegacyMarketMetaUsage) UnmarshalJSON(raw []byte) error {
	type alias LegacyMarketMetaUsage
	var v alias
	if e := decodeStrict(raw, &v); e != nil {
		return e
	}
	if e := legacyRequiredFields(raw, "owner", "client_id", "grants", "offset", "limit", "more_grants"); e != nil {
		return e
	}
	var fields map[string]json.RawMessage
	if json.Unmarshal(raw, &fields) != nil {
		return ErrInvalid
	}
	if _, ok := fields["client_budget"]; !ok {
		return ErrInvalid
	}
	*r = LegacyMarketMetaUsage(v)
	return nil
}
func (r *LegacyMarketMetaCall) UnmarshalJSON(raw []byte) error {
	type alias LegacyMarketMetaCall
	var v alias
	if e := decodeStrict(raw, &v); e != nil {
		return e
	}
	if e := legacyRequiredFields(raw, "call_id", "service_id", "tool_id", "version_id", "price_quota", "execution_status", "settlement_status", "created_at", "started_at", "finished_at"); e != nil {
		return e
	}
	*r = LegacyMarketMetaCall(v)
	return nil
}
func (r *LegacyMarketMetaCalls) UnmarshalJSON(raw []byte) error {
	type alias LegacyMarketMetaCalls
	var v alias
	if e := decodeStrict(raw, &v); e != nil {
		return e
	}
	if e := legacyRequiredFields(raw, "owner", "client_id", "calls", "offset", "limit", "more"); e != nil {
		return e
	}
	*r = LegacyMarketMetaCalls(v)
	return nil
}

type LegacyMarketOAuthClientsRequest struct {
	Offset        int  `json:"offset"`
	Limit         int  `json:"limit"`
	RequireManage bool `json:"require_manage"`
}
type LegacyMarketOAuthClient struct {
	ClientID string `json:"client_id"`
}
type LegacyMarketOAuthClients struct {
	Owner   Principal                 `json:"owner"`
	Clients []LegacyMarketOAuthClient `json:"clients"`
	Offset  int                       `json:"offset"`
	Limit   int                       `json:"limit"`
	More    bool                      `json:"more"`
}
type CoreLegacyMarketOAuthClients interface {
	ListLegacyMarketOAuthClients(context.Context, string, LegacyMarketOAuthClientsRequest) (LegacyMarketOAuthClients, error)
}

func (r *LegacyMarketOAuthClients) UnmarshalJSON(raw []byte) error {
	type alias LegacyMarketOAuthClients
	var v alias
	if e := decodeStrict(raw, &v); e != nil {
		return e
	}
	if e := legacyRequiredFields(raw, "owner", "clients", "offset", "limit", "more"); e != nil {
		return e
	}
	*r = LegacyMarketOAuthClients(v)
	return nil
}
func (r *LegacyMarketOAuthClient) UnmarshalJSON(raw []byte) error {
	type alias LegacyMarketOAuthClient
	var v alias
	if e := decodeStrict(raw, &v); e != nil {
		return e
	}
	if e := legacyRequiredFields(raw, "client_id"); e != nil {
		return e
	}
	*r = LegacyMarketOAuthClient(v)
	return nil
}
func (m *Module) LegacyListMarketOAuthClients(ctx context.Context, credential, account string, requireManage bool, offset, limit int) ([]LegacyMarketOAuthClient, error) {
	user, e := m.WebAuthorize(ctx, credential, account, "toolmarket.authorize")
	if e != nil {
		return nil, e
	}
	if !validWebPage(offset, limit) {
		return nil, ErrInvalid
	}
	port, ok := m.authority.(CoreLegacyMarketOAuthClients)
	if !ok || missingInterface(port) {
		return nil, ErrUnavailable
	}
	out, e := port.ListLegacyMarketOAuthClients(ctx, credential, LegacyMarketOAuthClientsRequest{Offset: offset, Limit: limit, RequireManage: requireManage})
	if e != nil {
		return nil, e
	}
	if out.Owner != (Principal{User: strconv.FormatInt(user, 10), Account: account}) || out.Offset != offset || out.Limit != limit || out.Clients == nil || len(out.Clients) > limit {
		return nil, ErrDenied
	}
	seen := map[string]bool{}
	for _, c := range out.Clients {
		if len(c.ClientID) <= 6 || !strings.HasPrefix(c.ClientID, "oauth:") || !legacyClientValid(c.ClientID) || seen[c.ClientID] {
			return nil, ErrDenied
		}
		seen[c.ClientID] = true
	}
	return out.Clients, nil
}
