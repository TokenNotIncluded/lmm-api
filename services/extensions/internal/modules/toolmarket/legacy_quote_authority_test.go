// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package toolmarket

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"math"
	"strings"
	"testing"
	"time"
)

func quoteTestHash(s string) string { h := sha256.Sum256([]byte(s)); return hex.EncodeToString(h[:]) }
func TestLegacyQuoteCanonicalPriceDigestV2(t *testing.T) {
	cases := []struct {
		price LegacyPricing
		wire  string
	}{
		{LegacyPricing{PriceQuota: 0}, `{"mode":"fixed","price_quota":0}`},
		{LegacyPricing{PriceQuota: 17}, `{"mode":"fixed","price_quota":17}`},
		{LegacyPricing{BillingMode: "input_tokens", InputTokenPriceQuota: 5, MaxInputTokens: 1000000}, `{"mode":"metered","rules":[{"metric":"input_tokens","rate_quota":5,"max_quantity":1000000}]}`},
		{LegacyPricing{BillingMode: "metered", BillingRules: []LegacyBillingRule{{Metric: "images", RateQuota: 3, MaxQuantity: 2}, {Metric: "input_characters", RateQuota: 5, MaxQuantity: 1001}}}, `{"mode":"metered","rules":[{"metric":"images","rate_quota":3,"max_quantity":2},{"metric":"input_characters","rate_quota":5,"max_quantity":1001}]}`},
	}
	for _, c := range cases {
		p, e := LegacyQuotePriceFromPricing(c.price)
		if e != nil {
			t.Fatal(e)
		}
		raw, e := json.Marshal(p)
		if e != nil || string(raw) != c.wire {
			t.Fatalf("canonical price got %s want %s err %v", raw, c.wire, e)
		}
		got, e := LegacyQuotePricingDigest(c.price)
		if e != nil || got != quoteTestHash(c.wire) {
			t.Fatalf("digest %s %v", got, e)
		}
		var back LegacyQuotePrice
		if e = json.Unmarshal(raw, &back); e != nil {
			t.Fatal(e)
		}
		if digest(back) != got {
			t.Fatal("wire digest changed")
		}
	}
}

type quoteCoreFixture struct {
	*webTestAuthority
	registered []LegacyCreditQuote
	approvals  []LegacyQuoteApproval
	fail       error
	corrupt    bool
	hook       func()
}

func (a *quoteCoreFixture) RegisterLegacyQuote(_ context.Context, _ string, in LegacyCreditQuote) (LegacyCreditQuote, error) {
	a.registered = append(a.registered, in)
	if a.hook != nil {
		a.hook()
	}
	if a.corrupt {
		in.Expires--
	}
	return in, a.fail
}
func (a *quoteCoreFixture) ApproveLegacyQuote(_ context.Context, credential string, in LegacyQuoteApproval) (LegacyQuoteApprovalReceipt, error) {
	a.approvals = append(a.approvals, in)
	if a.hook != nil {
		a.hook()
	}
	actor := credential
	if a.corrupt {
		actor = "2"
	}
	return LegacyQuoteApprovalReceipt{LegacyQuoteApproval: in, ApprovedBy: actor, ApprovedAt: 100}, a.fail
}
func quoteModule(t *testing.T) (*Module, *quoteCoreFixture) {
	t.Helper()
	a := &quoteCoreFixture{webTestAuthority: &webTestAuthority{}}
	d := legacyFixtureDB()
	d.Web.Services["service"] = WebServiceRecord{ID: "service", OwnerID: 1, Status: "draft", DraftVersionID: "version"}
	v := d.Web.Versions["version"]
	v.Version.Status = "draft"
	v.Version.ExecutionType = "remote"
	v.Version.Digest = strings.Repeat("a", 64)
	v.Digest = v.Version.Digest
	v.ValidationDigest = v.Digest
	v.RemoteDigests = map[string]string{"tool": strings.Repeat("b", 64)}
	v.Tools[0].PriceQuota = 0
	d.Web.Versions["version"] = v
	s := &Store{data: d}
	m, e := New(s, a, Config{WebOnly: true})
	if e != nil {
		t.Fatal(e)
	}
	m.now = func() time.Time { return time.Unix(100, 0) }
	t.Cleanup(func() { s.Close() })
	return m, a
}
func TestLegacyQuoteSnapshotRegistrationUsesFrozenDigest(t *testing.T) {
	m, a := quoteModule(t)
	ctx := context.Background()
	if e := m.legacyRegisterVersionQuotes(ctx, "1", "1", "service", "version"); e != nil {
		t.Fatal(e)
	}
	if len(a.registered) != 1 {
		t.Fatal(a.registered)
	}
	q := a.registered[0]
	priceHash := quoteTestHash(`{"mode":"fixed","price_quota":0}`)
	idWire := `["legacy-credit-quote-v2","version","tool","` + strings.Repeat("b", 64) + `","` + priceHash + `"]`
	if q.ID != quoteTestHash(idWire) || q.PricingDigest != priceHash || q.Expires != math.MaxInt64 {
		t.Fatalf("quote %+v", q)
	}
	if e := m.legacyRegisterVersionQuotes(ctx, "1", "1", "service", "version"); e != nil || a.registered[1].ID != q.ID {
		t.Fatal("registration changed identity on retry", e)
	}
	if len(a.approvals) != 0 {
		t.Fatal("register auto-approved quote")
	}
	if e := m.legacyRegisterVersionQuotes(ctx, "2", "2", "service", "version"); !errors.Is(e, ErrDenied) {
		t.Fatal("nonpublisher registered")
	}
}
func TestLegacyQuoteRejectsUnvalidatedOrChangedSnapshot(t *testing.T) {
	for _, kind := range []string{"validation", "remote", "provider", "receipt", "rotation"} {
		t.Run(kind, func(t *testing.T) {
			m, a := quoteModule(t)
			v := m.store.data.Web.Versions["version"]
			switch kind {
			case "validation":
				v.ValidationDigest = ""
			case "remote":
				v.RemoteDigests["tool"] = ""
			case "provider":
				v.Tools[0].ProviderPricing = &LegacyProviderPricing{}
			case "receipt":
				a.corrupt = true
			case "rotation":
				a.hook = func() {
					_ = m.store.update(func(d *database) error {
						cur := d.Web.Versions["version"]
						cur.ValidationDigest = ""
						d.Web.Versions["version"] = cur
						return nil
					})
				}
			}
			m.store.data.Web.Versions["version"] = v
			e := m.legacyRegisterVersionQuotes(context.Background(), "1", "1", "service", "version")
			if e == nil {
				t.Fatal("bad snapshot accepted")
			}
			if len(a.approvals) != 0 {
				t.Fatal("registration grants approval")
			}
		})
	}
}
func TestLegacyQuotePublicApprovalRequiresAdminAndExactScope(t *testing.T) {
	m, a := quoteModule(t)
	ctx := context.Background()
	if e := m.legacyApproveVersionQuotes(ctx, "1", "1", "service", "version", "public", true); !errors.Is(e, ErrDenied) {
		t.Fatal("publisher selfapproved public", e)
	}
	if e := m.legacyApproveVersionQuotes(ctx, "1", "1", "service", "version", "private", true); !errors.Is(e, ErrDenied) {
		t.Fatal("caller weakened visibility", e)
	}
	if e := m.legacyApproveVersionQuotes(ctx, "3", "3", "service", "version", "public", true); e != nil {
		t.Fatal(e)
	}
	first := a.approvals[0]
	if e := m.legacyApproveVersionQuotes(ctx, "3", "3", "service", "version", "public", true); e != nil || a.approvals[1] != first {
		t.Fatal("approval retry not stable", e)
	}
	a.fail = ErrConflict
	if e := m.legacyApproveVersionQuotes(ctx, "3", "3", "service", "version", "public", true); !errors.Is(e, ErrConflict) {
		t.Fatal("old op revived changed approval", e)
	}
	if m.store.data.Web.Services["service"].Status != "draft" {
		t.Fatal("authority helper published local service")
	}
}
func TestLegacyQuotePrivateApprovalOnlyOwnerAndFree(t *testing.T) {
	for _, price := range []int64{0, 1} {
		t.Run(string(rune('0'+price)), func(t *testing.T) {
			m, _ := quoteModule(t)
			v := m.store.data.Web.Versions["version"]
			v.Version.Visibility = "private"
			v.Tools[0].PriceQuota = price
			m.store.data.Web.Versions["version"] = v
			e := m.legacyApproveVersionQuotes(context.Background(), "1", "1", "service", "version", "private", true)
			if price == 0 && e != nil {
				t.Fatal(e)
			}
			if price > 0 && !errors.Is(e, ErrDenied) {
				t.Fatal("private paid accepted", e)
			}
			if e = m.legacyApproveVersionQuotes(context.Background(), "3", "3", "service", "version", "private", true); !errors.Is(e, ErrDenied) {
				t.Fatal("admin impersonated private owner", e)
			}
		})
	}
	m, _ := quoteModule(t)
	v := m.store.data.Web.Versions["version"]
	v.Version.Visibility = "shared"
	m.store.data.Web.Versions["version"] = v
	if e := m.legacyApproveVersionQuotes(context.Background(), "3", "3", "service", "version", "public", true); e != nil {
		t.Fatal("shared review failed", e)
	}
}
func TestLegacyQuoteWireRejectsMissingNullAndUnknownFields(t *testing.T) {
	for _, raw := range []string{`{"mode":"fixed"}`, `{"mode":"fixed","price_quota":null}`, `{"mode":"fixed","price_quota":0,"rules":[]}`, `{"mode":"metered","rules":[{"metric":"images","max_quantity":1}]}`} {
		var p LegacyQuotePrice
		if e := json.Unmarshal([]byte(raw), &p); e == nil {
			t.Fatal("malformed price accepted", raw)
		}
	}
	for _, raw := range []string{`{"operation_id":"op","quote_id":"quote","visibility":"private","approved_by":"1","approved_at":100}`, `{"operation_id":"op","quote_id":"quote","visibility":"private","approved":null,"approved_by":"1","approved_at":100}`} {
		var r LegacyQuoteApprovalReceipt
		if e := json.Unmarshal([]byte(raw), &r); e == nil {
			t.Fatal("missing approval became false", raw)
		}
	}
}

func TestLegacyQuotePrivateRejectsInvalidZeroRateMetering(t *testing.T) {
	m, _ := quoteModule(t)
	v := m.store.data.Web.Versions["version"]
	v.Version.Visibility = "private"
	v.Tools[0].BillingMode = "metered"
	v.Tools[0].BillingRules = []LegacyBillingRule{{Metric: "images", RateQuota: 0, MaxQuantity: 3}}
	m.store.data.Web.Versions["version"] = v
	if e := m.legacyApproveVersionQuotes(context.Background(), "1", "1", "service", "version", "private", true); !errors.Is(e, ErrInvalid) {
		t.Fatal("invalid zero rate accepted", e)
	}
}
func TestLegacyQuoteMissingProviderAndForgedApproverFailClosed(t *testing.T) {
	m, _, _ := webFixture(t)
	if e := m.legacyRegisterVersionQuotes(context.Background(), "1", "1", "service", "version"); !errors.Is(e, ErrUnavailable) {
		t.Fatal(e)
	}
	actual, a := quoteModule(t)
	a.corrupt = true
	if e := actual.legacyApproveVersionQuotes(context.Background(), "3", "3", "service", "version", "public", true); !errors.Is(e, ErrDenied) {
		t.Fatal("forged approver accepted", e)
	}
	actual, a = quoteModule(t)
	a.hook = func() { a.mu.Lock(); a.revoked = true; a.mu.Unlock() }
	if e := actual.legacyRegisterVersionQuotes(context.Background(), "1", "1", "service", "version"); !errors.Is(e, ErrAuth) {
		t.Fatal("revoked Session accepted after registration", e)
	}
	if actual.store.data.Web.Services["service"].Status != "draft" {
		t.Fatal("registration changed publication")
	}
}
