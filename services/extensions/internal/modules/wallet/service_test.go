// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package wallet

import (
	"context"
	"errors"
	"math"
	"strings"
	"testing"
)

type testRecords struct {
	token  string
	amount int64
}

func (r *testRecords) PutIfAbsent(_ context.Context, _ int64, i CreateInput, t string) (string, error) {
	if r.token != "" {
		if r.amount != i.AmountCredits {
			return "", ErrConflict
		}
		return r.token, nil
	}
	r.token = t
	r.amount = i.AmountCredits
	return t, nil
}
func (r *testRecords) Token(context.Context, int64, string) (string, error) { return r.token, nil }

type testCore struct {
	record    *testRecords
	seenToken string
	unknown   bool
	state     string
	user      int64
	calls     int
}

func (c *testCore) Balance(_ context.Context, s string) (Balance, error) {
	if s != "live-session" {
		return Balance{}, ErrUnauthorized
	}
	return Balance{OwnerUserID: c.user, AccountID: 5, CreditsPerUSD: CreditsPerUSD}, nil
}
func (c *testCore) Prepare(_ context.Context, _ string, i PrepareInput) (Transfer, error) {
	c.calls++
	if c.record.token == "" || c.record.token != i.Token {
		return Transfer{}, ErrConflict
	}
	if c.seenToken != "" && c.seenToken != i.Token {
		return Transfer{}, ErrConflict
	}
	c.seenToken = i.Token
	if c.unknown {
		c.unknown = false
		return Transfer{}, ErrUnknown
	}
	return Transfer{ID: 17, RequestKey: i.RequestKey, AmountCredits: i.AmountCredits, State: c.state}, nil
}
func (c *testCore) List(context.Context, string, int64) ([]Transfer, error) { return nil, nil }
func (c *testCore) Cancel(context.Context, string, int64) (Transfer, error) {
	return Transfer{ID: 17, RequestKey: strings.Repeat("k", 16), AmountCredits: 125, State: c.state}, nil
}
func (c *testCore) Inspect(context.Context, string, string) (Receipt, error) { return Receipt{}, nil }
func (c *testCore) Claim(context.Context, string, string) (Receipt, error)   { return Receipt{}, nil }
func setup(t *testing.T) (*Service, *testCore, *testRecords) {
	r := &testRecords{}
	c := &testCore{record: r, state: "awaiting_confirmation", user: 11}
	s, e := New(c, r, "https://wallet.example")
	if e != nil {
		t.Fatal(e)
	}
	return s, c, r
}
func TestUnknownRetryPreservesDurableBearerAndKey(t *testing.T) {
	s, c, r := setup(t)
	c.unknown = true
	in := CreateInput{125, strings.Repeat("k", 16)}
	_, e := s.Create(context.Background(), "live-session", in)
	if !errors.Is(e, ErrUnknown) {
		t.Fatal(e)
	}
	original := r.token
	t1, e := s.Create(context.Background(), "live-session", in)
	if e != nil || t1.ConfirmationURL != "https://wallet.example/core/v1/wallet/confirm/create/17" || t1.ShareURL != "" || r.token != original || c.calls != 2 {
		t.Fatalf("retry violated contract: %+v %v", t1, e)
	}
	c.state = "pending"
	t1, e = s.Create(context.Background(), "live-session", in)
	if e != nil || t1.ShareURL != "https://wallet.example/transfer#"+original || t1.ConfirmationURL != "" {
		t.Fatalf("confirmed response %+v %v", t1, e)
	}
	in.AmountCredits++
	_, e = s.Create(context.Background(), "live-session", in)
	if !errors.Is(e, ErrConflict) || c.calls != 3 {
		t.Fatal("amount mismatch reached authority", e)
	}
}
func TestEveryBusinessOperationAuthenticatesLiveSession(t *testing.T) {
	s, _, _ := setup(t)
	ctx := context.Background()
	token := strings.Repeat("a", 64)
	checks := []func() error{func() error { _, e := s.Balance(ctx, "api-key"); return e }, func() error { _, e := s.TopupLink(ctx, "api-key", 2); return e }, func() error { _, e := s.List(ctx, "api-key", 0); return e }, func() error { _, e := s.Create(ctx, "api-key", CreateInput{1, strings.Repeat("k", 16)}); return e }, func() error { _, e := s.Cancel(ctx, "api-key", 17); return e }, func() error { _, e := s.Inspect(ctx, "api-key", token); return e }, func() error { _, e := s.Claim(ctx, "api-key", token); return e }}
	for i, f := range checks {
		if e := f(); !errors.Is(e, ErrUnauthorized) {
			t.Fatalf("operation %d: %v", i, e)
		}
	}
}
func TestCancelRequiresIndependentBrowserConfirmation(t *testing.T) {
	s, c, _ := setup(t)
	c.state = "pending"
	v, e := s.Cancel(context.Background(), "live-session", 17)
	if e != nil || v.State != "pending" || v.ConfirmationURL != "https://wallet.example/core/v1/wallet/confirm/cancel/17" || v.ShareURL != "" {
		t.Fatalf("%+v %v", v, e)
	}
	c.state = "cancelled"
	v, e = s.Cancel(context.Background(), "live-session", 17)
	if e != nil || v.ConfirmationURL != "" {
		t.Fatal(v, e)
	}
}
func TestOnlyCanonicalOfficialOriginAndLegacyTopupAmount(t *testing.T) {
	s, c, r := setup(t)
	for _, bad := range []string{"http://wallet.example", "https://wallet.example/", "https://user@wallet.example", "https://wallet.example/?leak=x", "https://wallet.example#x"} {
		if _, e := New(c, r, bad); e == nil {
			t.Fatal(bad)
		}
	}
	v, e := s.TopupLink(context.Background(), "live-session", 1000000)
	if e != nil || v.URL != "https://wallet.example/wallet?topup_amount=1000000" || !v.PaymentConfirmationRequired || v.AmountUnit != "LEGACY" {
		t.Fatal(v, e)
	}
	for _, bad := range []int{0, -1, 1000001} {
		if _, e = s.TopupLink(context.Background(), "live-session", bad); !errors.Is(e, ErrInvalid) {
			t.Fatal(bad, e)
		}
	}
}

type queryCore struct {
	*testCore
	rows      []Transfer
	before    int64
	listCalls int
}

func (c *queryCore) List(_ context.Context, _ string, before int64) ([]Transfer, error) {
	c.before = before
	c.listCalls++
	return c.rows, nil
}
func (c *queryCore) Prepare(context.Context, string, PrepareInput) (Transfer, error) {
	panic("QueryTransfer invoked Prepare")
}
func (c *queryCore) Cancel(context.Context, string, int64) (Transfer, error) {
	panic("QueryTransfer invoked Cancel")
}
func TestQueryTransferIsExactOwnerCursorReadAndNeverMutation(t *testing.T) {
	base, core, records := setup(t)
	key := strings.Repeat("k", 16)
	records.token = strings.Repeat("a", 64)
	q := &queryCore{testCore: core, rows: []Transfer{{ID: 17, RequestKey: key, AmountCredits: 125, State: "pending"}, {ID: 16, RequestKey: key, AmountCredits: 125, State: "claimed"}}}
	s, err := New(q, records, base.origin)
	if err != nil {
		t.Fatal(err)
	}
	got, err := s.QueryTransfer(context.Background(), "live-session", 17)
	if err != nil || got.ID != 17 || got.State != "pending" || q.before != 18 || got.ShareURL == "" {
		t.Fatal(got, err, q.before)
	}
	cancellation, err := s.QueryCancellation(context.Background(), "live-session", 17)
	if err != nil || cancellation.State != "pending" || cancellation.ConfirmationURL != "https://wallet.example/core/v1/wallet/confirm/cancel/17" || cancellation.ShareURL != "" {
		t.Fatal(cancellation, err)
	}
	if _, err = s.QueryTransfer(context.Background(), "live-session", 15); err != ErrNotFound {
		t.Fatal("missing exact ID was accepted", err)
	}
	before := q.listCalls
	if _, err = s.QueryTransfer(context.Background(), "bad-session", 17); err != ErrUnauthorized || q.listCalls != before {
		t.Fatal("read bypassed live Session", err)
	}
	if _, err = s.QueryTransfer(context.Background(), "live-session", 0); err != ErrInvalid {
		t.Fatal(err)
	}
	q.rows = []Transfer{{ID: math.MaxInt64, RequestKey: key, AmountCredits: 125, State: "cancelled"}}
	got, err = s.QueryTransfer(context.Background(), "live-session", math.MaxInt64)
	if err != nil || got.ID != math.MaxInt64 || q.before != 0 {
		t.Fatal("overflow cursor", got, err, q.before)
	}
	got, err = s.QueryCancellation(context.Background(), "live-session", math.MaxInt64)
	if err != nil || got.State != "cancelled" || got.ConfirmationURL != "" {
		t.Fatal(got, err)
	}
	q.rows[0].State = "claimed"
	if _, err = s.QueryCancellation(context.Background(), "live-session", math.MaxInt64); err != ErrConflict {
		t.Fatal("claimed transfer appeared cancelled", err)
	}
}

func (c *testCore) QueryTransferByRequestKey(context.Context, string, string) (Transfer, error) {
	return Transfer{}, ErrNotFound
}

type queryKeyCore struct {
	*queryCore
	key   string
	value Transfer
	err   error
}

func (c *queryKeyCore) QueryTransferByRequestKey(_ context.Context, _ string, key string) (Transfer, error) {
	c.key = key
	return c.value, c.err
}
func (c *queryKeyCore) List(context.Context, string, int64) ([]Transfer, error) {
	panic("request-key recovery scanned a page")
}
func TestACKLostRequestKeyRecoveryDoesNotCreateOrScan(t *testing.T) {
	base, core, records := setup(t)
	key := strings.Repeat("f", 64)
	records.token = strings.Repeat("a", 64)
	q := &queryKeyCore{queryCore: &queryCore{testCore: core}, value: Transfer{ID: 22, RequestKey: key, AmountCredits: 125, State: "awaiting_confirmation"}}
	s, err := New(q, records, base.origin)
	if err != nil {
		t.Fatal(err)
	}
	got, err := s.QueryTransferByRequestKey(context.Background(), "live-session", key)
	if err != nil || got.ID != 22 || got.ConfirmationURL != "https://wallet.example/core/v1/wallet/confirm/create/22" || q.key != key {
		t.Fatal(got, err, q.key)
	}
	q.value.State = "pending"
	got, err = s.QueryTransferByRequestKey(context.Background(), "live-session", key)
	if err != nil || got.ShareURL != "https://wallet.example/transfer#"+records.token {
		t.Fatal(got, err)
	}
	q.err = ErrNotFound
	if _, err = s.QueryTransferByRequestKey(context.Background(), "live-session", key); err != ErrUnknown {
		t.Fatal("missing original row must remain UNKNOWN", err)
	}
	q.err = nil
	q.value.RequestKey = strings.Repeat("e", 64)
	if _, err = s.QueryTransferByRequestKey(context.Background(), "live-session", key); err != ErrUnknown {
		t.Fatal("different immutable key accepted", err)
	}
	if _, err = s.QueryTransferByRequestKey(context.Background(), "bad-session", key); err != ErrUnauthorized {
		t.Fatal("missing live session", err)
	}
	if _, err = s.QueryTransferByRequestKey(context.Background(), "live-session", "short"); err != ErrInvalid {
		t.Fatal(err)
	}
}

func TestMalformedRecipientReceiptsRemainUnknown(t *testing.T) {
	s, _, _ := setup(t)
	token := strings.Repeat("a", 64)
	if _, e := s.Inspect(context.Background(), "live-session", token); e != ErrUnknown {
		t.Fatal("empty inspect receipt accepted", e)
	}
	if _, e := s.Claim(context.Background(), "live-session", token); e != ErrUnknown {
		t.Fatal("empty claim receipt accepted", e)
	}
}

type readinessCore struct {
	*testCore
	calls int
	err   error
}

func (c *readinessCore) Ready(context.Context) error { c.calls++; return c.err }
func TestServiceReadinessRequiresRealAuthorityACKPort(t *testing.T) {
	s, core, records := setup(t)
	if e := s.Ready(context.Background()); e != ErrUnknown {
		t.Fatal("missing readiness became ready", e)
	}
	authority := &readinessCore{testCore: core, err: ErrForbidden}
	var e error
	s, e = New(authority, records, "https://wallet.example")
	if e != nil {
		t.Fatal(e)
	}
	if e = s.Ready(context.Background()); e != ErrForbidden || authority.calls != 1 {
		t.Fatal(e, authority.calls)
	}
	authority.err = nil
	if e = s.Ready(context.Background()); e != nil || authority.calls != 2 {
		t.Fatal(e, authority.calls)
	}
}
