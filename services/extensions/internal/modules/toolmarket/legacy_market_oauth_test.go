// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package toolmarket

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"errors"
	"strings"
	"testing"
)

type marketAuthFixture struct {
	*legacyCoreFixture
	got       CoreLegacyValidateRequest
	principal LegacyMarketPrincipal
	grants    []CoreLegacyConsentReceipt
}

func (f *marketAuthFixture) LegacyReadiness() LegacyCapabilities {
	return LegacyCapabilities{ProtocolMajor: 2, Unit: "credit_500k_usd", Features: append([]string(nil), legacyRequiredFeatures...)}
}
func (f *marketAuthFixture) ValidateLegacyToken(_ context.Context, _ string, in CoreLegacyValidateRequest) (CoreLegacyTokenReceipt, error) {
	f.got = in
	return f.validate, f.fail
}
func (f *marketAuthFixture) ListLegacyConsents(_ context.Context, _ string, in CoreLegacyListRequest) (CoreLegacyConsentList, error) {
	if f.grants == nil {
		return f.legacyCoreFixture.ListLegacyConsents(context.Background(), "", in)
	}
	return CoreLegacyConsentList{Owner: in.Owner, Rows: f.grants}, f.fail
}
func (f *marketAuthFixture) AuthorizeLegacyMarketOAuth(_ context.Context, in LegacyMarketOAuthRequest) (LegacyMarketPrincipal, error) {
	return f.principal, f.fail
}
func TestLegacyMarketAuthenticationDerivesOwnerAndKeepsPurpose(t *testing.T) {
	m, base := legacyCoreModule(t)
	base.validate = CoreLegacyTokenReceipt{CoreLegacyTokenRequest: CoreLegacyTokenRequest{CoreLegacyOperation: CoreLegacyOperation{Owner: Principal{User: "1", Account: "1"}}, ClientID: "client", CanInvoke: true, ExpiresAt: 200}, ID: "token", CreatedAt: 99}
	f := &marketAuthFixture{legacyCoreFixture: base}
	m.authority = f
	raw := "lmm_market_" + base64.RawURLEncoding.EncodeToString(make([]byte, 32))
	p, e := m.LegacyAuthenticateMarket(context.Background(), raw, []string{"market:discover", "market:invoke"})
	if e != nil || p.AccountID != "1" || f.got.Account != "" {
		t.Fatalf("caller selected owner: %+v %+v %v", p, f.got, e)
	}
	if _, e = m.LegacyAuthorizeMarketAction(context.Background(), raw, "2", "toolmarket.execute"); !errors.Is(e, ErrDenied) {
		t.Fatal("selected another payer", e)
	}
	for _, action := range []string{"toolmarket.authorize", "toolmarket.manage", "wallet.pay", "toolmarket.publish"} {
		if _, e = m.LegacyAuthorizeMarketAction(context.Background(), raw, "1", action); !errors.Is(e, ErrDenied) {
			t.Fatal(action, e)
		}
	}
	if _, e = m.LegacyAuthenticateMarket(context.Background(), raw, []string{"market:discover", "market:manage"}); !errors.Is(e, ErrLegacyMarketScope) {
		t.Fatal(e)
	}
	for _, owner := range []Principal{{User: "01", Account: "01"}, {User: "1", Account: "2"}, {User: "0", Account: "0"}} {
		f.validate.Owner = owner
		if _, e = m.LegacyAuthenticateClientToken(context.Background(), raw); !errors.Is(e, ErrDenied) {
			t.Fatal(owner, e)
		}
	}
}
func TestLegacyMarketOAuthClosedPrincipal(t *testing.T) {
	m, base := legacyCoreModule(t)
	f := &marketAuthFixture{legacyCoreFixture: base, principal: LegacyMarketPrincipal{UserID: "1", AccountID: "1", ClientID: "oauth:lmm-mcp-client", CredentialKind: "oauth", CredentialID: "family", ExpiresAt: 200, CanManage: true, Issuer: "https://example.test/api/oauth2/market", Resource: "https://example.test/mcp/market"}}
	m.authority = f
	if _, e := m.LegacyAuthenticateMarket(context.Background(), "lmm_at_token", []string{"market:discover", "market:manage"}); e != nil {
		t.Fatal(e)
	}
	for _, scopes := range [][]string{nil, {"market:invoke"}, {"market:discover", "market:discover"}, {"market:discover", "model.invoke"}} {
		if _, e := m.LegacyAuthenticateMarket(context.Background(), "lmm_at_token", scopes); !errors.Is(e, ErrInvalid) {
			t.Fatal(scopes, e)
		}
	}
	f.principal.CredentialKind = "session"
	if _, e := m.LegacyAuthenticateMarket(context.Background(), "lmm_at_token", []string{"market:discover"}); !errors.Is(e, ErrDenied) {
		t.Fatal(e)
	}
	f.principal.CredentialKind = "oauth"
	raw, _ := json.Marshal(f.principal)
	var p LegacyMarketPrincipal
	if e := json.Unmarshal(raw, &p); e != nil {
		t.Fatal(e)
	}
	for _, bad := range []string{strings.Replace(string(raw), `"can_invoke":false,`, "", 1), strings.Replace(string(raw), `"can_invoke":false`, `"can_invoke":null`, 1), strings.Replace(string(raw), `"can_invoke":false`, `"can_invoke":false,"can_invoke":true`, 1), strings.TrimSuffix(string(raw), "}") + `,"scope":"all"}`} {
		if json.Unmarshal([]byte(bad), &p) == nil {
			t.Fatal("accepted malformed Core identity", bad)
		}
	}
}
