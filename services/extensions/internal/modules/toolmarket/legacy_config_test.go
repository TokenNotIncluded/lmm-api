// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package toolmarket

import (
	"context"
	"encoding/json"
	"errors"
	"testing"
	"time"
)

type configCoreFixture struct {
	*webTestAuthority
	current        LegacyMarketConfig
	fail           error
	corrupt        string
	reads, updates int
	replay         bool
}

func (a *configCoreFixture) ReadLegacyConfig(context.Context) (LegacyMarketConfig, error) {
	a.reads++
	return a.current, a.fail
}
func (a *configCoreFixture) UpdateLegacyConfig(_ context.Context, _ string, in LegacyMarketConfigUpdate) (LegacyMarketConfigUpdateReceipt, error) {
	a.updates++
	out := LegacyMarketConfigUpdateReceipt{LegacyMarketConfig: LegacyMarketConfig{Enabled: in.Enabled, FeeBPS: in.FeeBPS, RecipientID: in.RecipientID, UpdatedBy: 3, UpdatedAt: 100, Version: a.current.Version + 1}, OperationID: in.OperationID}
	if a.replay {
		out.LegacyMarketConfig = a.current
	}
	switch a.corrupt {
	case "actor":
		out.UpdatedBy = 2
	case "version":
		out.Version = a.current.Version
	case "fee":
		out.FeeBPS++
	case "operation":
		out.OperationID = "other"
	case "time":
		out.UpdatedAt = 101
	}
	if a.fail == nil && a.corrupt == "" {
		a.current = out.LegacyMarketConfig
	}
	return out, a.fail
}
func configModule(t *testing.T) (*Module, *configCoreFixture) {
	t.Helper()
	a := &configCoreFixture{webTestAuthority: &webTestAuthority{}, current: LegacyMarketConfig{Version: 1}}
	store := &Store{data: emptyDB()}
	m, e := New(store, a, Config{WebOnly: true})
	if e != nil {
		t.Fatal(e)
	}
	m.now = func() time.Time { return time.Unix(100, 0) }
	t.Cleanup(func() { store.Close() })
	return m, a
}
func TestLegacyConfigReadsAuthoritativeDisabledInitialPolicy(t *testing.T) {
	m, a := configModule(t)
	got, e := m.LegacyReadConfig(context.Background())
	if e != nil || got.Enabled || got.Version != 1 || got.RecipientID != 0 || a.reads != 1 {
		t.Fatalf("%+v %v", got, e)
	}
	a.current = LegacyMarketConfig{Enabled: true, FeeBPS: 100, RecipientID: 3, UpdatedBy: 3, UpdatedAt: 100, Version: 2}
	got, e = m.LegacyReadConfig(context.Background())
	if e != nil || got != a.current {
		t.Fatalf("%+v %v", got, e)
	}
	a.fail = ErrUnavailable
	if _, e = m.LegacyReadConfig(context.Background()); !errors.Is(e, ErrUnavailable) {
		t.Fatal("fell back to local config", e)
	}
}
func TestLegacyConfigUpdateRequiresRootAndMatchesReadback(t *testing.T) {
	m, a := configModule(t)
	in := LegacyMarketConfigUpdate{Enabled: true, FeeBPS: 250, RecipientID: 3}
	ctx := context.Background()
	if _, e := m.LegacySetConfig(ctx, "1", "1", "op", in); !errors.Is(e, ErrDenied) || a.updates != 0 {
		t.Fatal("nonroot configured", e)
	}
	out, e := m.LegacySetConfig(ctx, "3", "3", "op", in)
	if e != nil || out.OperationID != "op" || out.Version != 2 || out.UpdatedBy != 3 || !out.Enabled {
		t.Fatalf("%+v %v", out, e)
	}
	a.replay = true
	repeat, e := m.LegacySetConfig(ctx, "3", "3", "op", in)
	if e != nil || repeat != out {
		t.Fatalf("replay %+v %v", repeat, e)
	}
	if m.store.data.LegacyAccess != nil {
		t.Fatal("Core config copied into Go business authority")
	}
}
func TestLegacyConfigRejectsForgedAuthorityAndStaleVersion(t *testing.T) {
	for _, kind := range []string{"actor", "version", "fee", "operation", "time"} {
		t.Run(kind, func(t *testing.T) {
			m, a := configModule(t)
			a.corrupt = kind
			if _, e := m.LegacySetConfig(context.Background(), "3", "3", "op", LegacyMarketConfigUpdate{Enabled: true, FeeBPS: 100, RecipientID: 3}); !errors.Is(e, ErrDenied) {
				t.Fatalf("bad %s accepted: %v", kind, e)
			}
		})
	}
	for _, bad := range []LegacyMarketConfig{{FeeBPS: 1, Version: 1}, {Enabled: true, RecipientID: 3, UpdatedBy: 3, UpdatedAt: 100, Version: 1}, {UpdatedBy: 3, UpdatedAt: 100, Version: 2}, {Enabled: true, Version: 1}, {FeeBPS: -1, Version: 1}, {FeeBPS: 10001, Version: 1}, {Version: 0}, {UpdatedBy: 3, UpdatedAt: 0, Version: 2}, {UpdatedBy: 3, UpdatedAt: 101, Version: 2}} {
		m, a := configModule(t)
		a.current = bad
		if _, e := m.LegacyReadConfig(context.Background()); !errors.Is(e, ErrDenied) {
			t.Fatalf("bad config %+v accepted %v", bad, e)
		}
	}
}
func TestLegacyConfigMissingPortAndInvalidInputFailClosed(t *testing.T) {
	m, _, _ := webFixture(t)
	if _, e := m.LegacyReadConfig(context.Background()); !errors.Is(e, ErrUnavailable) {
		t.Fatal(e)
	}
	for _, bad := range []LegacyMarketConfigUpdate{{RecipientID: 0}, {RecipientID: 3, FeeBPS: -1}, {RecipientID: 3, FeeBPS: 10001}, {RecipientID: 3, OperationID: "changed"}} {
		actual, a := configModule(t)
		if _, e := actual.LegacySetConfig(context.Background(), "3", "3", "op", bad); !errors.Is(e, ErrInvalid) || a.updates != 0 {
			t.Fatalf("invalid config accepted %+v %v", bad, e)
		}
	}
}
func TestLegacyConfigWireRequiresExplicitFields(t *testing.T) {
	for _, raw := range []string{`{"enabled":false,"fee_bps":0,"recipient_id":0,"updated_by":0,"updated_at":0}`, `{"fee_bps":0,"recipient_id":0,"updated_by":0,"updated_at":0,"version":1}`, `{"enabled":false,"fee_bps":0,"recipient_id":0,"updated_by":0,"updated_at":null,"version":1}`, `{"enabled":false,"fee_bps":0,"fee_bps":1,"recipient_id":0,"updated_by":0,"updated_at":0,"version":1}`} {
		var c LegacyMarketConfig
		if e := json.Unmarshal([]byte(raw), &c); e == nil {
			t.Fatal("missing/null/duplicate config field accepted", raw)
		}
	}
	want := LegacyMarketConfigUpdateReceipt{LegacyMarketConfig: LegacyMarketConfig{Enabled: true, FeeBPS: 100, RecipientID: 3, UpdatedBy: 3, UpdatedAt: 100, Version: 2}, OperationID: "op"}
	raw, e := json.Marshal(want)
	if e != nil {
		t.Fatal(e)
	}
	var got LegacyMarketConfigUpdateReceipt
	if e = json.Unmarshal(raw, &got); e != nil || got != want {
		t.Fatalf("flattened config receipt lost op: %+v %v", got, e)
	}
	var missing LegacyMarketConfigUpdateReceipt
	if e = json.Unmarshal([]byte(`{"enabled":false,"fee_bps":0,"recipient_id":0,"updated_by":0,"updated_at":0,"version":1}`), &missing); e == nil {
		t.Fatal("operation ID omitted")
	}
}
