// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package toolmarket

import (
	"context"
	"errors"
	"net/url"
	"strconv"
	"strings"
)

var ErrLegacyMarketScope = errors.New("market scope required")

// LegacyMarketPrincipal is resource-scoped identity, never a browser Session or
// general wallet authority. AccountID is the legacy personal owner key; Core
// resolves its ledger account separately.
type LegacyMarketPrincipal struct {
	UserID         string `json:"user_id"`
	AccountID      string `json:"account_id"`
	ClientID       string `json:"client_id"`
	CredentialKind string `json:"credential_kind"`
	CredentialID   string `json:"credential_id"`
	AuthVersion    int64  `json:"auth_version"`
	ExpiresAt      int64  `json:"expires_at_unix"`
	CanInvoke      bool   `json:"can_invoke"`
	CanManage      bool   `json:"can_manage"`
	Issuer         string `json:"issuer"`
	Resource       string `json:"resource"`
}
type LegacyMarketOAuthRequest struct {
	Token          string   `json:"token"`
	RequiredScopes []string `json:"required_scopes"`
}
type CoreLegacyMarketOAuth interface {
	AuthorizeLegacyMarketOAuth(context.Context, LegacyMarketOAuthRequest) (LegacyMarketPrincipal, error)
}

func (p *LegacyMarketPrincipal) UnmarshalJSON(raw []byte) error {
	type alias LegacyMarketPrincipal
	var next alias
	if e := decodeStrict(raw, &next); e != nil {
		return e
	}
	if e := legacyRequiredFields(raw, "user_id", "account_id", "client_id", "credential_kind", "credential_id", "auth_version", "expires_at_unix", "can_invoke", "can_manage", "issuer", "resource"); e != nil {
		return e
	}
	*p = LegacyMarketPrincipal(next)
	return nil
}
func legacyMarketScopes(scopes []string) bool {
	if len(scopes) < 1 || len(scopes) > 3 {
		return false
	}
	seen := map[string]bool{}
	for _, s := range scopes {
		if seen[s] || s != "market:discover" && s != "market:invoke" && s != "market:manage" {
			return false
		}
		seen[s] = true
	}
	return seen["market:discover"]
}
func legacyCanonicalUser(user, account string) bool {
	n, e := strconv.ParseInt(user, 10, 64)
	return e == nil && n > 0 && strconv.FormatInt(n, 10) == user && account == user
}
func legacyAbsoluteResource(s string) bool {
	u, e := url.Parse(s)
	return e == nil && (u.Scheme == "https" || u.Scheme == "http") && u.Host != "" && u.User == nil && u.RawQuery == "" && u.Fragment == ""
}

// LegacyAuthenticateClientToken derives its owner from Core. The caller cannot
// select a payer/account before introspection.
func (m *Module) LegacyAuthenticateClientToken(ctx context.Context, raw string) (LegacyMetaSubject, error) {
	if m == nil || !legacyRawToken(raw) {
		return LegacyMetaSubject{}, ErrDenied
	}
	port, ok := m.authority.(CoreLegacyAccess)
	if !ok || missingInterface(port) {
		return LegacyMetaSubject{}, ErrUnavailable
	}
	r, e := port.ValidateLegacyToken(ctx, raw, CoreLegacyValidateRequest{})
	if e != nil {
		return LegacyMetaSubject{}, e
	}
	if !legacyCanonicalUser(r.Owner.User, r.Owner.Account) || !r.Owner.valid() || r.OperationID != "" || r.Raw != "" || r.Replayed || !validID.MatchString(r.ID) || !validLegacyPersonalClient(r.ClientID) || r.AuthVersion < 0 || r.CreatedAt <= 0 || r.CreatedAt > m.now().Unix() || r.ExpiresAt <= m.now().Unix() || r.RevokedAt != 0 {
		return LegacyMetaSubject{}, ErrDenied
	}
	u, _ := strconv.ParseInt(r.Owner.User, 10, 64)
	return LegacyMetaSubject{UserID: u, ClientID: r.ClientID, CredentialKind: "personal", CredentialID: r.ID, AuthVersion: r.AuthVersion, ExpiresAt: r.ExpiresAt, CanInvoke: r.CanInvoke, CanManage: r.CanManage}, nil
}

func (m *Module) LegacyAuthenticateMarket(ctx context.Context, raw string, scopes []string) (LegacyMarketPrincipal, error) {
	if m == nil || !legacyMarketScopes(scopes) {
		return LegacyMarketPrincipal{}, ErrInvalid
	}
	var p LegacyMarketPrincipal
	if strings.HasPrefix(raw, "lmm_market_") {
		s, e := m.LegacyAuthenticateClientToken(ctx, raw)
		if e != nil {
			return p, e
		}
		u := strconv.FormatInt(s.UserID, 10)
		p = LegacyMarketPrincipal{UserID: u, AccountID: u, ClientID: s.ClientID, CredentialKind: s.CredentialKind, CredentialID: s.CredentialID, AuthVersion: s.AuthVersion, ExpiresAt: s.ExpiresAt, CanInvoke: s.CanInvoke, CanManage: s.CanManage}
	} else {
		if !strings.HasPrefix(raw, "lmm_at_") || len(raw) > 4096 || strings.ContainsAny(raw, " \t\r\n") {
			return p, ErrDenied
		}
		port, ok := m.authority.(CoreLegacyMarketOAuth)
		if !ok || missingInterface(port) {
			return p, ErrUnavailable
		}
		var e error
		p, e = port.AuthorizeLegacyMarketOAuth(ctx, LegacyMarketOAuthRequest{Token: raw, RequiredScopes: append([]string(nil), scopes...)})
		if e != nil {
			return LegacyMarketPrincipal{}, e
		}
		if !legacyCanonicalUser(p.UserID, p.AccountID) || p.CredentialKind != "oauth" || !strings.HasPrefix(p.ClientID, "oauth:") || !legacyClientValid(p.ClientID) || len(p.ClientID) <= 6 || !validID.MatchString(p.CredentialID) || p.AuthVersion < 0 || p.ExpiresAt <= m.now().Unix() || !legacyAbsoluteResource(p.Issuer) || !legacyAbsoluteResource(p.Resource) {
			return LegacyMarketPrincipal{}, ErrDenied
		}
	}
	for _, s := range scopes {
		if s == "market:invoke" && !p.CanInvoke || s == "market:manage" && !p.CanManage {
			return LegacyMarketPrincipal{}, ErrLegacyMarketScope
		}
	}
	return p, nil
}

// Only the two resource actions can enter shared read/execution plumbing.
// Delegation, management, publishing and wallet purposes have separate gates.
func (m *Module) LegacyAuthorizeMarketAction(ctx context.Context, raw, account, action string) (int64, error) {
	scopes := []string{"market:discover"}
	switch action {
	case "toolmarket.read":
	case "toolmarket.execute":
		scopes = append(scopes, "market:invoke")
	default:
		return 0, ErrDenied
	}
	p, e := m.LegacyAuthenticateMarket(ctx, raw, scopes)
	if e != nil {
		return 0, e
	}
	if p.AccountID != account {
		return 0, ErrDenied
	}
	u, _ := strconv.ParseInt(p.UserID, 10, 64)
	return u, nil
}
