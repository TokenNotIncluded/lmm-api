// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
// Business-record semantics adapted from fixed main 3948a0a:
// apps/api-go/model/tool_market_{access,clients,client_records,meta_delegation}.go.
package toolmarket

import (
	"sort"
	"strings"
)

// LegacyAccessRecords contains consent and display records only. These records
// never authenticate a bearer or reserve funds. Core must independently enforce
// identity, revocation, authorization and every spending ceiling before dispatch.
type LegacyAccessRecords struct {
	Grants         map[string]LegacyConsentGrant `json:"grants"`
	Tokens         map[string]LegacyTokenRecord  `json:"tokens"`
	Budgets        map[string]LegacyBudgetRecord `json:"budgets"`
	Policies       map[string]LegacyMetaPolicy   `json:"policies"`
	RemovedClients map[string]bool               `json:"removed_clients"`
}
type LegacyConsentGrant struct {
	QuoteID          string `json:"quote_id,omitempty"`
	FrozenPriceQuota int64  `json:"frozen_price_quota,omitempty"`
	RulesHash        string `json:"rules_hash,omitempty"`
	ID               string `json:"id"`
	UserID           int64  `json:"user_id"`
	ClientID         string `json:"client_id"`
	ToolID           string `json:"tool_id"`
	VersionID        string `json:"version_id"`
	MaxPriceQuota    int64  `json:"max_price_quota"`
	MaxTotalQuota    int64  `json:"max_total_quota"`
	MaxCalls         int64  `json:"max_calls"`
	CreatedAt        int64  `json:"created_at"`
	ExpiresAt        int64  `json:"expires_at"`
	RevokedAt        int64  `json:"revoked_at"`
	Hidden           bool   `json:"hidden,omitempty"`
}

// This is a reference to a Core-owned connection, never a token or token digest.
type LegacyTokenRecord struct {
	CanInvoke bool   `json:"can_invoke"`
	CanManage bool   `json:"can_manage"`
	ID        string `json:"id"`
	UserID    int64  `json:"user_id"`
	ClientID  string `json:"client_id"`
	Name      string `json:"name"`
	CreatedAt int64  `json:"created_at"`
	ExpiresAt int64  `json:"expires_at"`
	RevokedAt int64  `json:"revoked_at"`
	Hidden    bool   `json:"hidden,omitempty"`
}
type LegacyBudgetRecord struct {
	UserID     int64  `json:"user_id"`
	Scope      string `json:"scope"`
	ScopeID    string `json:"scope_id"`
	LimitQuota int64  `json:"limit_quota"`
}
type LegacyMetaPolicy struct {
	UserID         int64  `json:"user_id"`
	ClientID       string `json:"client_id"`
	CredentialKind string `json:"credential_kind"`
	CredentialID   string `json:"credential_id"`
	AuthVersion    int64  `json:"auth_version"`
	Enabled        bool   `json:"enabled"`
	MaxTotalQuota  int64  `json:"max_total_quota"`
	ExpiresAt      int64  `json:"expires_at"`
	UpdatedAt      int64  `json:"updated_at"`
}

// LegacyMetaSubject must come from current Core introspection, never request JSON.
// OAuth eligibility requires issuer/resource/scope/family checks in Core.
type LegacyMetaSubject struct {
	UserID                                 int64
	ClientID, CredentialKind, CredentialID string
	AuthVersion, ExpiresAt                 int64
	CanInvoke, CanManage                   bool
}
type LegacyClientChange struct {
	ClientID      string `json:"client_id"`
	TokensRevoked int64  `json:"tokens_revoked"`
	GrantsRevoked int64  `json:"grants_revoked"`
	TokensHidden  int64  `json:"tokens_hidden"`
	GrantsHidden  int64  `json:"grants_hidden"`
	ToolsUnloaded int64  `json:"tools_unloaded"`
}

func legacyAccess(d *database) *LegacyAccessRecords {
	if d.LegacyAccess == nil {
		d.LegacyAccess = &LegacyAccessRecords{}
	}
	a := d.LegacyAccess
	if a.Grants == nil {
		a.Grants = map[string]LegacyConsentGrant{}
	}
	if a.Tokens == nil {
		a.Tokens = map[string]LegacyTokenRecord{}
	}
	if a.Budgets == nil {
		a.Budgets = map[string]LegacyBudgetRecord{}
	}
	if a.Policies == nil {
		a.Policies = map[string]LegacyMetaPolicy{}
	}
	if a.RemovedClients == nil {
		a.RemovedClients = map[string]bool{}
	}
	return a
}
func legacyClientValid(id string) bool {
	return id != "" && len(id) <= 128 && strings.TrimSpace(id) == id
}
func legacyQuotaValid(n int64) bool                       { return n >= 0 && n <= 9007199254740991 }
func legacyClientKey(user int64, client string) string    { return digest([]any{user, client}) }
func legacyBudgetKey(user int64, scope, id string) string { return digest([]any{user, scope, id}) }
func legacyPolicyKey(s LegacyMetaSubject) string {
	return digest([]any{s.UserID, s.CredentialKind, s.CredentialID})
}
func legacyLiveTool(d *database, user int64, toolID, versionID string) (WebTool, error) {
	if d.Web == nil {
		return WebTool{}, ErrMissing
	}
	v, ok := d.Web.Versions[versionID]
	s, exists := d.Web.Services[v.Version.ServiceID]
	if !ok || !exists || s.LiveVersionID != versionID || !webVisible(s, v, user) {
		return WebTool{}, ErrMissing
	}
	for _, t := range v.Tools {
		if t.ToolID == toolID {
			return t, nil
		}
	}
	return WebTool{}, ErrMissing
}

// Caller runs this in the module transaction after live owner authorization.
func legacyCreateGrant(d *database, user, now int64, in LegacyConsentGrant) (LegacyConsentGrant, error) {
	if user <= 0 || !legacyClientValid(in.ClientID) || !legacyQuotaValid(in.MaxPriceQuota) || !legacyQuotaValid(in.MaxTotalQuota) || in.MaxCalls < 1 || in.MaxCalls > 1000000 || in.ExpiresAt <= now {
		return LegacyConsentGrant{}, ErrInvalid
	}
	if _, err := legacyLiveTool(d, user, in.ToolID, in.VersionID); err != nil {
		return LegacyConsentGrant{}, err
	}
	a := legacyAccess(d)
	if len(a.Grants) >= 50000 {
		return LegacyConsentGrant{}, ErrLimit
	}
	id, err := randomID()
	if err != nil {
		return LegacyConsentGrant{}, ErrUnavailable
	}
	out := LegacyConsentGrant{ID: id, UserID: user, ClientID: in.ClientID, ToolID: in.ToolID, VersionID: in.VersionID, MaxPriceQuota: in.MaxPriceQuota, MaxTotalQuota: in.MaxTotalQuota, MaxCalls: in.MaxCalls, CreatedAt: now, ExpiresAt: in.ExpiresAt}
	a.Grants[id] = out
	return out, nil
}
func legacyRevokeGrant(d *database, user, now int64, id string) error {
	a := legacyAccess(d)
	g, ok := a.Grants[id]
	if !ok || g.UserID != user {
		return ErrMissing
	}
	if g.RevokedAt == 0 {
		g.RevokedAt = now
		a.Grants[id] = g
	}
	return nil
}
func legacyHideRecord(d *database, user int64, id, kind string) error {
	a := legacyAccess(d)
	switch kind {
	case "grant":
		g, ok := a.Grants[id]
		if !ok || g.UserID != user {
			return ErrMissing
		}
		if g.RevokedAt == 0 {
			return ErrConflict
		}
		g.Hidden = true
		a.Grants[id] = g
	case "token":
		t, ok := a.Tokens[id]
		if !ok || t.UserID != user {
			return ErrMissing
		}
		if t.RevokedAt == 0 {
			return ErrConflict
		}
		t.Hidden = true
		a.Tokens[id] = t
	default:
		return ErrInvalid
	}
	return nil
}

// Disconnect/remove must follow successful Core connection/grant revocation.
// It preserves budgets, executions and all settlement history.
func legacyChangeClient(d *database, user, now int64, client string, remove bool) (LegacyClientChange, error) {
	out := LegacyClientChange{ClientID: client}
	if user <= 0 || !legacyClientValid(client) || client == "web-market" || strings.HasPrefix(client, "oauth:") {
		return out, ErrInvalid
	}
	a := legacyAccess(d)
	found := false
	if remove {
		for _, t := range a.Tokens {
			if t.UserID == user && t.ClientID == client && t.RevokedAt == 0 {
				return out, ErrConflict
			}
		}
		for _, g := range a.Grants {
			if g.UserID == user && g.ClientID == client && g.RevokedAt == 0 {
				return out, ErrConflict
			}
		}
	}
	for id, t := range a.Tokens {
		if t.UserID != user || t.ClientID != client {
			continue
		}
		found = true
		if remove {
			if !t.Hidden {
				t.Hidden = true
				out.TokensHidden++
			}
		} else if t.RevokedAt == 0 {
			t.RevokedAt = now
			out.TokensRevoked++
		}
		a.Tokens[id] = t
	}
	for id, g := range a.Grants {
		if g.UserID != user || g.ClientID != client {
			continue
		}
		found = true
		if remove {
			if !g.Hidden {
				g.Hidden = true
				out.GrantsHidden++
			}
		} else if g.RevokedAt == 0 {
			g.RevokedAt = now
			out.GrantsRevoked++
		}
		a.Grants[id] = g
	}
	if d.Web != nil {
		for id, i := range d.Web.Installations {
			if i.UserID == user && i.ClientID == client {
				delete(d.Web.Installations, id)
				out.ToolsUnloaded++
				found = true
			}
		}
	}
	key := legacyClientKey(user, client)
	if remove {
		if !found && !a.RemovedClients[key] {
			return out, ErrMissing
		}
		a.RemovedClients[key] = true
	}
	return out, nil
}

// Core must check existing holds/spending and atomically enforce the new limit.
// Go only records the requested constraint; it never fabricates balance counters.
func legacySetBudget(d *database, user int64, in LegacyBudgetRecord) error {
	if user <= 0 || !legacyQuotaValid(in.LimitQuota) {
		return ErrInvalid
	}
	switch in.Scope {
	case "account":
		if in.ScopeID != "" {
			return ErrInvalid
		}
	case "client":
		if !legacyClientValid(in.ScopeID) {
			return ErrInvalid
		}
	case "tool":
		if !validID.MatchString(in.ScopeID) {
			return ErrInvalid
		}
	default:
		return ErrInvalid
	}
	in.UserID = user
	legacyAccess(d).Budgets[legacyBudgetKey(user, in.Scope, in.ScopeID)] = in
	return nil
}
func legacySubjectValid(s LegacyMetaSubject, now int64) bool {
	return s.UserID > 0 && legacyClientValid(s.ClientID) && s.CredentialID != "" && len(s.CredentialID) <= 128 && s.AuthVersion >= 0 && s.ExpiresAt > now && s.CanInvoke && s.CanManage && (s.CredentialKind == "personal" || s.CredentialKind == "oauth" && strings.HasPrefix(s.ClientID, "oauth:"))
}
func legacySetMetaPolicy(d *database, s LegacyMetaSubject, now int64, in LegacyMetaPolicy) (LegacyMetaPolicy, error) {
	if !legacySubjectValid(s, now) {
		return LegacyMetaPolicy{}, ErrDenied
	}
	if !legacyQuotaValid(in.MaxTotalQuota) || !in.Enabled && (in.MaxTotalQuota != 0 || in.ExpiresAt != 0) {
		return LegacyMetaPolicy{}, ErrInvalid
	}
	a := legacyAccess(d)
	if in.Enabled {
		if in.ExpiresAt == 0 {
			in.ExpiresAt = s.ExpiresAt
		}
		if in.ExpiresAt <= now || in.ExpiresAt > s.ExpiresAt {
			return LegacyMetaPolicy{}, ErrInvalid
		}
		if b, ok := a.Budgets[legacyBudgetKey(s.UserID, "client", s.ClientID)]; ok {
			in.MaxTotalQuota = min(in.MaxTotalQuota, b.LimitQuota)
		}
	}
	if in.Enabled {
		if err := legacySetBudget(d, s.UserID, LegacyBudgetRecord{Scope: "client", ScopeID: s.ClientID, LimitQuota: in.MaxTotalQuota}); err != nil {
			return LegacyMetaPolicy{}, err
		}
	}
	out := LegacyMetaPolicy{UserID: s.UserID, ClientID: s.ClientID, CredentialKind: s.CredentialKind, CredentialID: s.CredentialID, AuthVersion: s.AuthVersion, Enabled: in.Enabled, MaxTotalQuota: in.MaxTotalQuota, ExpiresAt: in.ExpiresAt, UpdatedAt: now}
	a.Policies[legacyPolicyKey(s)] = out
	return out, nil
}
func legacyMetaPolicy(d *database, s LegacyMetaSubject, now int64) (LegacyMetaPolicy, error) {
	if !legacySubjectValid(s, now) {
		return LegacyMetaPolicy{}, ErrDenied
	}
	p, ok := legacyAccess(d).Policies[legacyPolicyKey(s)]
	if !ok || !p.Enabled || p.UserID != s.UserID || p.ClientID != s.ClientID || p.CredentialKind != s.CredentialKind || p.CredentialID != s.CredentialID || p.AuthVersion != s.AuthVersion || p.ExpiresAt <= now || p.ExpiresAt > s.ExpiresAt {
		return LegacyMetaPolicy{}, ErrDenied
	}
	return p, nil
}
func legacyListGrants(d *database, user int64) []LegacyConsentGrant {
	rows := []LegacyConsentGrant{}
	for _, g := range legacyAccess(d).Grants {
		if g.UserID == user && !g.Hidden {
			rows = append(rows, g)
		}
	}
	sort.Slice(rows, func(i, j int) bool {
		if rows[i].CreatedAt == rows[j].CreatedAt {
			return rows[i].ID < rows[j].ID
		}
		return rows[i].CreatedAt > rows[j].CreatedAt
	})
	return rows
}

// legacyAuthorizeMeta only narrows stored consent; callers must first apply the
// same intent in Core. Repeated calls cannot refresh allowance or erase holds.
func legacyAuthorizeMeta(d *database, s LegacyMetaSubject, now int64, in LegacyConsentGrant) (LegacyConsentGrant, error) {
	policy, err := legacyMetaPolicy(d, s, now)
	if err != nil {
		return LegacyConsentGrant{}, err
	}
	if in.ToolID == "" || in.VersionID == "" || !legacyQuotaValid(in.MaxPriceQuota) || !legacyQuotaValid(in.MaxTotalQuota) || in.MaxCalls < 1 || in.MaxCalls > 1000000 || in.ExpiresAt <= now {
		return LegacyConsentGrant{}, ErrInvalid
	}
	if in.MaxPriceQuota > in.MaxTotalQuota || in.MaxTotalQuota > policy.MaxTotalQuota || in.ExpiresAt > policy.ExpiresAt {
		return LegacyConsentGrant{}, ErrLimit
	}
	b, ok := legacyAccess(d).Budgets[legacyBudgetKey(s.UserID, "client", s.ClientID)]
	if !ok || in.MaxTotalQuota > b.LimitQuota {
		return LegacyConsentGrant{}, ErrLimit
	}
	tool, err := legacyLiveTool(d, s.UserID, in.ToolID, in.VersionID)
	if err != nil {
		return LegacyConsentGrant{}, err
	}
	if tool.PriceQuota > in.MaxPriceQuota {
		return LegacyConsentGrant{}, ErrLimit
	}
	var existing *LegacyConsentGrant
	for _, g := range legacyAccess(d).Grants {
		if g.UserID == s.UserID && g.ClientID == s.ClientID && g.ToolID == in.ToolID && g.VersionID == in.VersionID && g.RevokedAt == 0 && g.ExpiresAt > now {
			if existing == nil || g.CreatedAt > existing.CreatedAt || g.CreatedAt == existing.CreatedAt && g.ID > existing.ID {
				copy := g
				existing = &copy
			}
		}
	}
	if existing != nil {
		g := *existing
		if in.MaxPriceQuota > g.MaxPriceQuota || in.MaxTotalQuota > g.MaxTotalQuota || in.MaxCalls > g.MaxCalls || in.ExpiresAt > g.ExpiresAt {
			return LegacyConsentGrant{}, ErrLimit
		}
		g.MaxPriceQuota = in.MaxPriceQuota
		g.MaxTotalQuota = in.MaxTotalQuota
		g.MaxCalls = in.MaxCalls
		g.ExpiresAt = in.ExpiresAt
		legacyAccess(d).Grants[g.ID] = g
		return g, nil
	}
	in.ClientID = s.ClientID
	return legacyCreateGrant(d, s.UserID, now, in)
}
func legacyTightenClientBudget(d *database, s LegacyMetaSubject, now, limit int64) error {
	p, err := legacyMetaPolicy(d, s, now)
	if err != nil {
		return err
	}
	if !legacyQuotaValid(limit) {
		return ErrInvalid
	}
	key := legacyBudgetKey(s.UserID, "client", s.ClientID)
	b, ok := legacyAccess(d).Budgets[key]
	if !ok || limit > p.MaxTotalQuota || limit > b.LimitQuota {
		return ErrLimit
	}
	b.LimitQuota = limit
	legacyAccess(d).Budgets[key] = b
	return nil
}
func legacyTightenToolBudget(d *database, s LegacyMetaSubject, now int64, id, tool, version string, limit int64) error {
	p, err := legacyMetaPolicy(d, s, now)
	if err != nil {
		return err
	}
	if !legacyQuotaValid(limit) {
		return ErrInvalid
	}
	g, ok := legacyAccess(d).Grants[id]
	if !ok || g.UserID != s.UserID || g.ClientID != s.ClientID || g.ToolID != tool || g.VersionID != version || g.RevokedAt != 0 || g.ExpiresAt <= now {
		return ErrMissing
	}
	if limit > p.MaxTotalQuota || limit > g.MaxTotalQuota {
		return ErrLimit
	}
	g.MaxTotalQuota = limit
	legacyAccess(d).Grants[id] = g
	return nil
}

// Record an already-created Core connection for owner UI. Caller must revoke in
// Core first; changing this display row cannot revoke any live credential.
func legacyRecordToken(d *database, user int64, in LegacyTokenRecord) error {
	if user <= 0 || in.ID == "" || len(in.ID) > 128 || !legacyClientValid(in.ClientID) || in.ClientID == "web-market" || strings.HasPrefix(in.ClientID, "oauth:") || len(in.Name) > 128 || in.CreatedAt <= 0 || in.ExpiresAt <= in.CreatedAt {
		return ErrInvalid
	}
	a := legacyAccess(d)
	if prior, ok := a.Tokens[in.ID]; ok {
		if prior.UserID != user || prior.ClientID != in.ClientID || prior.CreatedAt != in.CreatedAt {
			return ErrConflict
		}
		if prior.RevokedAt != 0 && in.RevokedAt == 0 {
			return ErrConflict
		}
		in.Hidden = prior.Hidden
	}
	if len(a.Tokens) >= 50000 {
		if _, ok := a.Tokens[in.ID]; !ok {
			return ErrLimit
		}
	}
	in.UserID = user
	a.Tokens[in.ID] = in
	return nil
}
func legacyListTokens(d *database, user int64) []LegacyTokenRecord {
	rows := []LegacyTokenRecord{}
	for _, t := range legacyAccess(d).Tokens {
		if t.UserID == user && !t.Hidden {
			rows = append(rows, t)
		}
	}
	sort.Slice(rows, func(i, j int) bool {
		if rows[i].CreatedAt == rows[j].CreatedAt {
			return rows[i].ID < rows[j].ID
		}
		return rows[i].CreatedAt > rows[j].CreatedAt
	})
	return rows
}
