// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package toolmarket

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"errors"
	"strings"
	"testing"
	"time"
)

// This adversarial boundary fake verifies Go rejects malformed authority replies.
// It is not a substitute for independent Core PostgreSQL acceptance.
type legacyCoreFixture struct {
	*webTestAuthority
	fail     error
	corrupt  string
	replay   bool
	validate CoreLegacyTokenReceipt
	calls    int
}

func (f *legacyCoreFixture) CreateLegacyConsent(_ context.Context, _ string, in CoreLegacyConsentRequest) (CoreLegacyConsentReceipt, error) {
	f.calls++
	out := CoreLegacyConsentReceipt{CoreLegacyConsentRequest: in, ID: "core-consent", QuoteID: "core-quote", FrozenPriceQuota: 10, RulesHash: strings.Repeat("a", 64), CreatedAt: 99}
	if f.corrupt == "owner" {
		out.Owner.User = "2"
	}
	if f.corrupt == "price" {
		out.FrozenPriceQuota = in.MaxPriceQuota + 1
	}
	if f.corrupt == "rules" {
		out.RulesHash = "unknown"
	}
	if f.corrupt == "counters" {
		out.ReservedQuota = in.MaxTotalQuota
		out.SpentQuota = 1
	}
	return out, f.fail
}
func (f *legacyCoreFixture) RevokeLegacyConsent(_ context.Context, _ string, in CoreLegacyIDRequest) (CoreLegacyRevocationReceipt, error) {
	f.calls++
	return CoreLegacyRevocationReceipt{CoreLegacyIDRequest: in, RevokedAt: 100}, f.fail
}
func (f *legacyCoreFixture) SetLegacyBudget(_ context.Context, _ string, in CoreLegacyBudgetRequest) (CoreLegacyBudgetReceipt, error) {
	f.calls++
	out := CoreLegacyBudgetReceipt{CoreLegacyBudgetRequest: in, ReservedQuota: 5, SpentQuota: 5}
	if f.corrupt == "owner" {
		out.Owner.User = "2"
	}
	if f.corrupt == "counters" {
		out.ReservedQuota = in.LimitQuota
		out.SpentQuota = 1
	}
	return out, f.fail
}
func (f *legacyCoreFixture) CreateLegacyToken(_ context.Context, _ string, in CoreLegacyTokenRequest) (CoreLegacyTokenReceipt, error) {
	f.calls++
	out := CoreLegacyTokenReceipt{CoreLegacyTokenRequest: in, ID: "core-token", AuthVersion: 3, CreatedAt: 99, Raw: "lmm_market_" + base64.RawURLEncoding.EncodeToString(make([]byte, 32)), Replayed: f.replay}
	if f.replay {
		out.Raw = ""
	}
	if f.corrupt == "owner" {
		out.Owner.User = "2"
	}
	return out, f.fail
}
func (f *legacyCoreFixture) RevokeLegacyToken(_ context.Context, _ string, in CoreLegacyIDRequest) (CoreLegacyRevocationReceipt, error) {
	f.calls++
	return CoreLegacyRevocationReceipt{CoreLegacyIDRequest: in, RevokedAt: 100}, f.fail
}
func (f *legacyCoreFixture) ValidateLegacyToken(_ context.Context, _ string, _ CoreLegacyValidateRequest) (CoreLegacyTokenReceipt, error) {
	f.calls++
	return f.validate, f.fail
}
func (f *legacyCoreFixture) DisconnectLegacyClient(_ context.Context, _ string, in CoreLegacyClientRequest) (CoreLegacyDisconnectReceipt, error) {
	f.calls++
	out := CoreLegacyDisconnectReceipt{CoreLegacyClientRequest: in, RevokedAt: 100, TokensRevoked: 1, GrantsRevoked: 1}
	if f.corrupt == "owner" {
		out.Owner.Account = "2"
	}
	return out, f.fail
}
func legacyCoreModule(t *testing.T) (*Module, *legacyCoreFixture) {
	t.Helper()
	a := &legacyCoreFixture{webTestAuthority: &webTestAuthority{}}
	store := &Store{data: legacyFixtureDB()}
	m, e := New(store, a, Config{WebOnly: true})
	if e != nil {
		t.Fatal(e)
	}
	m.now = func() time.Time { return time.Unix(100, 0) }
	t.Cleanup(func() { store.Close() })
	return m, a
}
func TestLegacyCoreMissingProviderFailsClosed(t *testing.T) {
	m, _, _ := webFixture(t)
	if _, e := m.LegacySetBudget(context.Background(), "1", "1", "op", LegacyBudgetRecord{Scope: "account", LimitQuota: 100}); !errors.Is(e, ErrUnavailable) {
		t.Fatal(e)
	}
}
func TestLegacyCoreConsentReadbackAndRevocation(t *testing.T) {
	for _, bad := range []string{"owner", "price", "rules", "counters"} {
		t.Run(bad, func(t *testing.T) {
			m, a := legacyCoreModule(t)
			a.corrupt = bad
			in := legacyGrantInput("Client")
			in.UserID = 999
			if _, e := m.LegacyCreateConsent(context.Background(), "1", "1", "op", in); !errors.Is(e, ErrDenied) {
				t.Fatal(e)
			}
			if m.store.data.LegacyAccess != nil {
				t.Fatal("bad receipt persisted")
			}
		})
	}
	m, a := legacyCoreModule(t)
	in := legacyGrantInput("Client")
	in.UserID = 999
	got, e := m.LegacyCreateConsent(context.Background(), "1", "1", "op", in)
	if e != nil || got.Owner.User != "1" {
		t.Fatalf("%+v %v", got, e)
	}
	if g := m.store.data.LegacyAccess.Grants[got.ID]; g.UserID != 1 || g.QuoteID != "core-quote" || g.RulesHash != strings.Repeat("a", 64) {
		t.Fatal("frozen authority readback not persisted")
	}
	a.fail = ErrUnavailable
	if e = m.LegacyRevokeConsent(context.Background(), "1", "1", "revoke", got.ID); !errors.Is(e, ErrUnavailable) {
		t.Fatal(e)
	}
	if m.store.data.LegacyAccess.Grants[got.ID].RevokedAt != 0 {
		t.Fatal("local revoke before Core ACK")
	}
	a.fail = nil
	if e = m.LegacyRevokeConsent(context.Background(), "1", "1", "revoke", got.ID); e != nil {
		t.Fatal(e)
	}
	if m.store.data.LegacyAccess.Grants[got.ID].RevokedAt != 100 {
		t.Fatal("revocation absent")
	}
}
func TestLegacyCoreBudgetRejectsImpossibleCounters(t *testing.T) {
	m, a := legacyCoreModule(t)
	a.corrupt = "counters"
	in := LegacyBudgetRecord{UserID: 999, Scope: "account", LimitQuota: 100}
	if _, e := m.LegacySetBudget(context.Background(), "1", "1", "op", in); !errors.Is(e, ErrDenied) {
		t.Fatal(e)
	}
	if m.store.data.LegacyAccess != nil {
		t.Fatal("bad budget persisted")
	}
	a.corrupt = "owner"
	if _, e := m.LegacySetBudget(context.Background(), "1", "1", "op", in); !errors.Is(e, ErrDenied) {
		t.Fatal(e)
	}
	a.corrupt = ""
	got, e := m.LegacySetBudget(context.Background(), "1", "1", "op", in)
	if e != nil || got.ReservedQuota != 5 || got.SpentQuota != 5 {
		t.Fatalf("%+v %v", got, e)
	}
	for _, b := range m.store.data.LegacyAccess.Budgets {
		if b.UserID != 1 {
			t.Fatal("caller selected owner")
		}
	}
}
func TestLegacyCoreTokenSecretOnlyReturnedOnceAndNeverStored(t *testing.T) {
	m, a := legacyCoreModule(t)
	in := CoreLegacyTokenRequest{ClientID: "Client", CanInvoke: true, CanManage: false, ExpiresAt: 200}
	out, e := m.LegacyCreateToken(context.Background(), "1", "1", "op", in)
	if e != nil || !legacyRawToken(out.Raw) {
		t.Fatalf("%+v %v", out, e)
	}
	raw := out.Raw
	stored, _ := json.Marshal(m.store.data)
	if strings.Contains(string(stored), raw) || strings.Contains(string(stored), "lmm_market_") {
		t.Fatal("secret stored in Go")
	}
	a.replay = true
	out, e = m.LegacyCreateToken(context.Background(), "1", "1", "op", in)
	if e != nil || out.Raw != "" || !out.Replayed || out.ID != "core-token" {
		t.Fatalf("replay %+v %v", out, e)
	}
	out.OperationID = ""
	out.Replayed = false
	a.validate = out
	subject, e := m.LegacyValidateClientToken(context.Background(), raw, "1")
	if e != nil || subject.CredentialID != "core-token" || subject.CanManage {
		t.Fatalf("scope: %+v %v", subject, e)
	}
	a.validate.Owner.Account = "2"
	if _, e = m.LegacyValidateClientToken(context.Background(), raw, "1"); !errors.Is(e, ErrDenied) {
		t.Fatal("forged owner accepted")
	}
}
func TestLegacyCoreDisconnectNeedsCoreAcknowledgement(t *testing.T) {
	m, a := legacyCoreModule(t)
	d := &m.store.data
	legacyAccess(d).Tokens["token"] = LegacyTokenRecord{ID: "token", UserID: 1, ClientID: "Client"}
	d.Web.Installations["i"] = WebInstallation{UserID: 1, ClientID: "Client", ToolID: "tool"}
	a.fail = ErrUnavailable
	if _, e := m.LegacyDisconnectClient(context.Background(), "1", "1", "op", "Client"); !errors.Is(e, ErrUnavailable) {
		t.Fatal(e)
	}
	if len(d.Web.Installations) != 1 || d.LegacyAccess.Tokens["token"].RevokedAt != 0 {
		t.Fatal("disconnected before Core ACK")
	}
	a.fail = nil
	a.corrupt = "owner"
	if _, e := m.LegacyDisconnectClient(context.Background(), "1", "1", "op", "Client"); !errors.Is(e, ErrDenied) {
		t.Fatal(e)
	}
	if len(m.store.data.Web.Installations) != 1 {
		t.Fatal("forged ACK unloaded")
	}
	a.corrupt = ""
	out, e := m.LegacyDisconnectClient(context.Background(), "1", "1", "op", "Client")
	if e != nil || out.ToolsUnloaded != 1 || out.TokensRevoked != 1 {
		t.Fatalf("%+v %v", out, e)
	}
	if len(m.store.data.Web.Installations) != 0 || m.store.data.LegacyAccess.Tokens["token"].RevokedAt != 100 {
		t.Fatal("Core confirmed disconnect absent")
	}
}

func (f *legacyCoreFixture) ListLegacyConsents(_ context.Context, _ string, in CoreLegacyListRequest) (CoreLegacyConsentList, error) {
	r, _ := f.CreateLegacyConsent(context.Background(), "", CoreLegacyConsentRequest{CoreLegacyOperation: CoreLegacyOperation{Owner: in.Owner}, ClientID: "Client", ToolID: "tool", VersionID: "version", MaxPriceQuota: 10, MaxTotalQuota: 100, MaxCalls: 10, ExpiresAt: 200})
	return CoreLegacyConsentList{Owner: in.Owner, Rows: []CoreLegacyConsentReceipt{r}}, f.fail
}
func (f *legacyCoreFixture) ListLegacyBudgets(_ context.Context, _ string, in CoreLegacyListRequest) (CoreLegacyBudgetList, error) {
	r, _ := f.SetLegacyBudget(context.Background(), "", CoreLegacyBudgetRequest{CoreLegacyOperation: CoreLegacyOperation{Owner: in.Owner}, Scope: "account", LimitQuota: 100})
	return CoreLegacyBudgetList{Owner: in.Owner, Rows: []CoreLegacyBudgetReceipt{r}}, f.fail
}
func (f *legacyCoreFixture) ListLegacyTokens(_ context.Context, _ string, in CoreLegacyListRequest) (CoreLegacyTokenList, error) {
	r, _ := f.CreateLegacyToken(context.Background(), "", CoreLegacyTokenRequest{CoreLegacyOperation: CoreLegacyOperation{Owner: in.Owner}, ClientID: "Client", CanInvoke: true, ExpiresAt: 200})
	r.Raw = ""
	r.Replayed = false
	if f.corrupt == "secret" {
		r.Raw = "forbidden-secret"
	}
	return CoreLegacyTokenList{Owner: in.Owner, Rows: []CoreLegacyTokenReceipt{r}}, f.fail
}
func TestLegacyCoreListsUseAuthorityCountersAndNeverTokenSecrets(t *testing.T) {
	m, a := legacyCoreModule(t)
	ctx := context.Background()
	rows, e := m.LegacyListBudgets(ctx, "1", "1", 0, 20)
	if e != nil || len(rows.Rows) != 1 || rows.Rows[0].ReservedQuota != 5 || rows.Rows[0].SpentQuota != 5 {
		t.Fatalf("%+v %v", rows, e)
	}
	if _, e = m.LegacyListConsents(ctx, "1", "1", 0, 20); e != nil {
		t.Fatal(e)
	}
	if _, e = m.LegacyListTokens(ctx, "1", "1", 0, 20); e != nil {
		t.Fatal(e)
	}
	a.corrupt = "counters"
	if _, e = m.LegacyListBudgets(ctx, "1", "1", 0, 20); !errors.Is(e, ErrDenied) {
		t.Fatal(e)
	}
	if _, e = m.LegacyListConsents(ctx, "1", "1", 0, 20); !errors.Is(e, ErrDenied) {
		t.Fatal(e)
	}
	a.corrupt = "owner"
	if _, e = m.LegacyListTokens(ctx, "1", "1", 0, 20); !errors.Is(e, ErrDenied) {
		t.Fatal(e)
	}
	a.corrupt = "secret"
	if _, e = m.LegacyListTokens(ctx, "1", "1", 0, 20); !errors.Is(e, ErrDenied) {
		t.Fatal("list exposed secret")
	}
	a.corrupt = ""
	a.fail = ErrUnavailable
	if _, e = m.LegacyListBudgets(ctx, "1", "1", 0, 20); !errors.Is(e, ErrUnavailable) {
		t.Fatal("fell back to local zero counters")
	}
}

func TestLegacyCoreListMirrorsKeepHiddenAndRejectResurrection(t *testing.T) {
	m, a := legacyCoreModule(t)
	ctx := context.Background()
	grants, e := m.LegacyListConsents(ctx, "1", "1", 0, 20)
	if e != nil {
		t.Fatal(e)
	}
	tokens, e := m.LegacyListTokens(ctx, "1", "1", 0, 20)
	if e != nil {
		t.Fatal(e)
	}
	if len(m.store.data.LegacyAccess.Grants) != 1 || len(m.store.data.LegacyAccess.Tokens) != 1 {
		t.Fatal("Core records not mirrored")
	}
	g := grants.Rows[0]
	g.RevokedAt = 100
	tok := tokens.Rows[0]
	tok.RevokedAt = 100
	if e = m.store.update(func(d *database) error {
		if e := legacyMirrorCoreConsents(d, 1, []CoreLegacyConsentReceipt{g}); e != nil {
			return e
		}
		if e := legacyMirrorCoreTokens(d, 1, []CoreLegacyTokenReceipt{tok}); e != nil {
			return e
		}
		if e := legacyHideRecord(d, 1, g.ID, "grant"); e != nil {
			return e
		}
		return legacyHideRecord(d, 1, tok.ID, "token")
	}); e != nil {
		t.Fatal(e)
	}
	if e = m.store.update(func(d *database) error {
		if e := legacyMirrorCoreConsents(d, 1, []CoreLegacyConsentReceipt{g}); e != nil {
			return e
		}
		return legacyMirrorCoreTokens(d, 1, []CoreLegacyTokenReceipt{tok})
	}); e != nil {
		t.Fatal(e)
	}
	if !m.store.data.LegacyAccess.Grants[g.ID].Hidden || !m.store.data.LegacyAccess.Tokens[tok.ID].Hidden {
		t.Fatal("hidden refs resurfaced")
	}
	if e = m.store.update(func(d *database) error { return legacyMirrorCoreConsents(d, 1, grants.Rows) }); !errors.Is(e, ErrConflict) {
		t.Fatal("stale active grant resurrected")
	}
	if e = m.store.update(func(d *database) error { return legacyMirrorCoreTokens(d, 1, tokens.Rows) }); !errors.Is(e, ErrConflict) {
		t.Fatal("stale active token resurrected")
	}
	g.Owner.User = "2"
	if e = m.store.update(func(d *database) error { return legacyMirrorCoreConsents(d, 1, []CoreLegacyConsentReceipt{g}) }); !errors.Is(e, ErrDenied) {
		t.Fatal("owner alias accepted")
	}
	a.corrupt = ""
	stored, _ := json.Marshal(m.store.data)
	if strings.Contains(string(stored), "reserved_quota") || strings.Contains(string(stored), "spent_quota") {
		t.Fatal("Go became authoritative counters store")
	}
}

func TestLegacyCoreWireDoesNotInventZeroCounters(t *testing.T) {
	for _, raw := range []string{
		`{"owner":{"user":"1","account":"1"},"operation_id":"op","scope":"account","scope_id":"","limit_quota":100,"reserved_quota":0}`,
		`{"owner":{"user":"1","account":"1"},"operation_id":"op","scope":"account","scope_id":"","limit_quota":100,"reserved_quota":null,"spent_quota":0}`,
		`{"owner":{"user":"1","account":"1"},"operation_id":"op","scope":"account","scope_id":"","limit_quota":100,"reserved_quota":0,"reserved_quota":1,"spent_quota":0}`,
	} {
		var receipt CoreLegacyBudgetReceipt
		if e := json.Unmarshal([]byte(raw), &receipt); e == nil {
			t.Fatalf("missing/null/duplicate counter accepted: %s", raw)
		}
	}
	good := CoreLegacyBudgetReceipt{CoreLegacyBudgetRequest: CoreLegacyBudgetRequest{CoreLegacyOperation: CoreLegacyOperation{Owner: Principal{User: "1", Account: "1"}, OperationID: "op"}, Scope: "account", LimitQuota: 100}}
	raw, e := json.Marshal(good)
	if e != nil {
		t.Fatal(e)
	}
	var got CoreLegacyBudgetReceipt
	if e = json.Unmarshal(raw, &got); e != nil || got != good {
		t.Fatalf("explicit zero counters rejected: %+v %v", got, e)
	}
}
