// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package toolmarket

import (
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"testing"
)

func legacyFixtureDB() database {
	d := emptyDB()
	w := webState(&d)
	w.Services["service"] = WebServiceRecord{ID: "service", OwnerID: 1, Status: "published", LiveVersionID: "version"}
	w.Versions["version"] = webSnapshot{Version: WebVersion{ID: "version", ServiceID: "service", Status: "published", Visibility: "public"}, Tools: []WebTool{{ToolID: "tool", VersionID: "version", Name: "echo", PriceQuota: 10}}}
	return d
}
func legacyGrantInput(client string) LegacyConsentGrant {
	return LegacyConsentGrant{ClientID: client, ToolID: "tool", VersionID: "version", MaxPriceQuota: 10, MaxTotalQuota: 100, MaxCalls: 10, ExpiresAt: 200}
}
func TestLegacyAccessConsentRecords(t *testing.T) {
	d := legacyFixtureDB()
	g, err := legacyCreateGrant(&d, 1, 100, legacyGrantInput("Case"))
	if err != nil {
		t.Fatal(err)
	}
	if err = legacyHideRecord(&d, 1, g.ID, "grant"); !errors.Is(err, ErrConflict) {
		t.Fatalf("active record hidden: %v", err)
	}
	if err = legacyRevokeGrant(&d, 2, 101, g.ID); !errors.Is(err, ErrMissing) {
		t.Fatalf("other owner: %v", err)
	}
	if err = legacyRevokeGrant(&d, 1, 101, g.ID); err != nil {
		t.Fatal(err)
	}
	if err = legacyHideRecord(&d, 1, g.ID, "grant"); err != nil {
		t.Fatal(err)
	}
	b, _ := json.Marshal(d)
	var reopened database
	if err = json.Unmarshal(b, &reopened); err != nil {
		t.Fatal(err)
	}
	if len(legacyListGrants(&reopened, 1)) != 0 || !reopened.LegacyAccess.Grants[g.ID].Hidden {
		t.Fatal("hidden revocation lost on snapshot")
	}
	if _, err = legacyCreateGrant(&d, 1, 100, legacyGrantInput(" Case")); !errors.Is(err, ErrInvalid) {
		t.Fatal(err)
	}
	d.Web.Services["service"] = WebServiceRecord{ID: "service", Status: "paused", LiveVersionID: "version"}
	if _, err = legacyCreateGrant(&d, 1, 100, legacyGrantInput("Case")); !errors.Is(err, ErrMissing) {
		t.Fatalf("paused service accepted: %v", err)
	}
}
func TestLegacyClientDisconnectIsExactAndPreservesBudgets(t *testing.T) {
	d := legacyFixtureDB()
	a := legacyAccess(&d)
	a.Tokens["a"] = LegacyTokenRecord{ID: "a", UserID: 1, ClientID: "Case"}
	a.Tokens["b"] = LegacyTokenRecord{ID: "b", UserID: 1, ClientID: "case"}
	g, err := legacyCreateGrant(&d, 1, 100, legacyGrantInput("Case"))
	if err != nil {
		t.Fatal(err)
	}
	if err = legacySetBudget(&d, 1, LegacyBudgetRecord{Scope: "client", ScopeID: "Case", LimitQuota: 40}); err != nil {
		t.Fatal(err)
	}
	d.Web.Installations["load"] = WebInstallation{UserID: 1, ClientID: "Case", ToolID: "tool"}
	if _, err = legacyChangeClient(&d, 1, 101, "Case", true); !errors.Is(err, ErrConflict) {
		t.Fatal(err)
	}
	c, err := legacyChangeClient(&d, 1, 101, "Case", false)
	if err != nil || c.TokensRevoked != 1 || c.GrantsRevoked != 1 || c.ToolsUnloaded != 1 {
		t.Fatalf("disconnect: %+v %v", c, err)
	}
	if a.Tokens["b"].RevokedAt != 0 || a.Grants[g.ID].RevokedAt != 101 || len(a.Budgets) != 1 {
		t.Fatal("other client or budgets changed")
	}
	c, err = legacyChangeClient(&d, 1, 102, "Case", false)
	if err != nil || c.TokensRevoked+c.GrantsRevoked+c.ToolsUnloaded != 0 {
		t.Fatal("disconnect not idempotent")
	}
	c, err = legacyChangeClient(&d, 1, 102, "Case", true)
	if err != nil || c.TokensHidden != 1 || c.GrantsHidden != 1 {
		t.Fatalf("remove: %+v %v", c, err)
	}
	if _, e := legacyChangeClient(&d, 1, 102, "web-market", false); !errors.Is(e, ErrInvalid) {
		t.Fatal("web client disconnected")
	}
	if _, err = legacyChangeClient(&d, 1, 102, "oauth:Case", false); !errors.Is(err, ErrInvalid) {
		t.Fatal("oauth falsely disconnected")
	}
}
func TestLegacyMetaPolicyBindsCredentialAndOnlyTightens(t *testing.T) {
	d := legacyFixtureDB()
	s := LegacyMetaSubject{UserID: 1, ClientID: "client", CredentialKind: "personal", CredentialID: "core-token", AuthVersion: 3, ExpiresAt: 250, CanInvoke: true, CanManage: true}
	if err := legacySetBudget(&d, 1, LegacyBudgetRecord{Scope: "client", ScopeID: s.ClientID, LimitQuota: 80}); err != nil {
		t.Fatal(err)
	}
	p, err := legacySetMetaPolicy(&d, s, 100, LegacyMetaPolicy{Enabled: true, MaxTotalQuota: 100, ExpiresAt: 220})
	if err != nil || p.MaxTotalQuota != 80 {
		t.Fatalf("policy %+v %v", p, err)
	}
	in := legacyGrantInput("attacker")
	in.MaxTotalQuota = 70
	g, err := legacyAuthorizeMeta(&d, s, 100, in)
	if err != nil || g.ClientID != "client" {
		t.Fatalf("grant %+v %v", g, err)
	}
	in.MaxTotalQuota = 60
	g2, err := legacyAuthorizeMeta(&d, s, 101, in)
	if err != nil || g2.ID != g.ID || g2.MaxTotalQuota != 60 {
		t.Fatalf("tighten %+v %v", g2, err)
	}
	in.MaxTotalQuota = 70
	if _, err = legacyAuthorizeMeta(&d, s, 102, in); !errors.Is(err, ErrLimit) {
		t.Fatalf("replenished consent: %v", err)
	}
	if err = legacyTightenClientBudget(&d, s, 102, 50); err != nil {
		t.Fatal(err)
	}
	if err = legacyTightenClientBudget(&d, s, 102, 51); !errors.Is(err, ErrLimit) {
		t.Fatal(err)
	}
	old := s
	s.AuthVersion++
	if _, err = legacyMetaPolicy(&d, s, 103); !errors.Is(err, ErrDenied) {
		t.Fatal("old credential policy survives auth change")
	}
	s = old
	s.CredentialID = "other"
	if _, err = legacyMetaPolicy(&d, s, 103); !errors.Is(err, ErrDenied) {
		t.Fatal("credential identity changed")
	}
	s = old
	s.CanManage = false
	if _, err = legacyMetaPolicy(&d, s, 103); !errors.Is(err, ErrDenied) {
		t.Fatal("manage scope absent")
	}
}
func TestLegacyAccessEncryptedReopen(t *testing.T) {
	dir := t.TempDir()
	if err := os.Chmod(dir, 0700); err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(dir, "access.enc")
	key := make([]byte, 32)
	store, err := InitStore(path, key)
	if err != nil {
		t.Fatal(err)
	}
	if err = store.update(func(d *database) error {
		*d = legacyFixtureDB()
		g, e := legacyCreateGrant(d, 1, 100, legacyGrantInput("Case"))
		if e != nil {
			return e
		}
		if e = legacyRevokeGrant(d, 1, 101, g.ID); e != nil {
			return e
		}
		return legacyHideRecord(d, 1, g.ID, "grant")
	}); err != nil {
		t.Fatal(err)
	}
	if err = store.Close(); err != nil {
		t.Fatal(err)
	}
	store, err = OpenStore(path, key)
	if err != nil {
		t.Fatal(err)
	}
	defer store.Close()
	if err = store.view(func(d database) error {
		if len(d.LegacyAccess.Grants) != 1 || len(legacyListGrants(&d, 1)) != 0 {
			t.Fatal("hidden grant not durable")
		}
		return nil
	}); err != nil {
		t.Fatal(err)
	}
}

func TestLegacyAccessTransactionRollback(t *testing.T) {
	store := &Store{data: legacyFixtureDB()}
	defer store.Close()
	if err := store.update(func(d *database) error {
		if _, e := legacyCreateGrant(d, 1, 100, legacyGrantInput("client")); e != nil {
			return e
		}
		return ErrConflict
	}); !errors.Is(err, ErrConflict) {
		t.Fatal(err)
	}
	if err := store.view(func(d database) error {
		if d.LegacyAccess != nil {
			t.Fatal("failed transaction wrote consent")
		}
		return nil
	}); err != nil {
		t.Fatal(err)
	}
}
func TestLegacyTokenRecordsNeverResurrectHiddenCredentials(t *testing.T) {
	d := legacyFixtureDB()
	in := LegacyTokenRecord{ID: "core-reference", ClientID: "Client", Name: "desktop", CreatedAt: 100, ExpiresAt: 200, RevokedAt: 110}
	if err := legacyRecordToken(&d, 1, in); err != nil {
		t.Fatal(err)
	}
	if err := legacyHideRecord(&d, 1, in.ID, "token"); err != nil {
		t.Fatal(err)
	}
	if err := legacyRecordToken(&d, 1, in); err != nil {
		t.Fatal(err)
	}
	if len(legacyListTokens(&d, 1)) != 0 {
		t.Fatal("hidden token resurfaced")
	}
	in.RevokedAt = 0
	if err := legacyRecordToken(&d, 1, in); !errors.Is(err, ErrConflict) {
		t.Fatal("credential resurrected")
	}
	in.RevokedAt = 110
	if err := legacyRecordToken(&d, 2, in); !errors.Is(err, ErrConflict) {
		t.Fatal("other owner's record overwritten")
	}
	for _, b := range []LegacyBudgetRecord{{Scope: "account", ScopeID: "unexpected"}, {Scope: "client", ScopeID: " x"}, {Scope: "unknown"}, {Scope: "account", LimitQuota: -1}} {
		if err := legacySetBudget(&d, 1, b); !errors.Is(err, ErrInvalid) {
			t.Fatalf("invalid budget accepted: %+v %v", b, err)
		}
	}
}
