// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package store

import (
	"context"
	"encoding/json"
	"errors"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
)

// These are development regressions with synthetic persistence/authority/funds.
// They do not prove PostgreSQL durability or a real Core ledger movement.
type a06RefundIndex struct {
	*memoryRepo
	shop       string
	failWrites atomic.Bool
}

type a06RefundTx struct {
	Tx
	wrote *bool
}

func (tx a06RefundTx) Put(kind, id string, value any) error {
	*tx.wrote = true
	return tx.Tx.Put(kind, id, value)
}
func (r *a06RefundIndex) Within(ctx context.Context, shop string, fn func(Tx) error) error {
	return r.memoryRepo.Within(ctx, shop, func(tx Tx) error {
		wrote := false
		if err := fn(a06RefundTx{tx, &wrote}); err != nil {
			return err
		}
		if wrote && r.failWrites.Swap(false) {
			r.failCommit.Store(true)
		}
		return nil
	})
}

func (r *a06RefundIndex) FindOrder(ctx context.Context, id string) (LocatedOrder, error) {
	var o Order
	err := r.Within(ctx, r.shop, func(tx Tx) error { return tx.Get("orders", id, &o) })
	if err != nil {
		return LocatedOrder{}, err
	}
	return LocatedOrder{ShopID: r.shop, ProductID: o.Legacy.ProductID, Order: o}, err
}
func (*a06RefundIndex) FindProduct(context.Context, string) (LocatedProduct, error) {
	return LocatedProduct{}, ErrUnavailable
}
func (*a06RefundIndex) ListPublished(context.Context, int64, string, int, int) ([]LocatedProduct, error) {
	return nil, ErrUnavailable
}

func a06RefundSetup(t *testing.T) *a06StateFixture {
	t.Helper()
	f := a06StateSetup(t)
	_, err := f.change(buyerToken, "pay", "")
	must(t, err)
	f.s, err = New(&a06RefundIndex{memoryRepo: f.repo, shop: f.shop}, f.auth, f.funds)
	must(t, err)
	return f
}
func a06RefundCall(f *a06StateFixture, token, action, child string, body any) (any, error) {
	var raw json.RawMessage
	if body != nil {
		var err error
		raw, err = json.Marshal(body)
		if err != nil {
			return nil, err
		}
	}
	actor := map[string]int64{buyerToken: 2, sellerToken: 1, otherToken: 3}[token]
	return f.s.Legacy(context.Background(), token, Account{"personal", actor}, LegacyRequest{Action: action, ID: f.id, ChildID: child, Body: raw})
}
func a06RefundRequest() map[string]any {
	return map[string]any{"request_key": "synthetic-refund", "reason": "Synthetic full refund", "mode": "full"}
}
func a06RefundOpen(t *testing.T, f *a06StateFixture) string {
	t.Helper()
	v, err := a06RefundCall(f, buyerToken, "request_refund", "", a06RefundRequest())
	must(t, err)
	return v.(map[string]any)["id"].(string)
}

func TestA06LegacyFullRefundReturnsOriginalPrincipalAndRetainsDelivery(t *testing.T) {
	f := a06RefundSetup(t)
	_, err := f.s.FinalizeDelivery(context.Background(), f.shop, f.id)
	must(t, err)
	id := a06RefundOpen(t, f)
	v, err := a06RefundCall(f, buyerToken, "request_refund", "", a06RefundRequest())
	must(t, err)
	if v.(map[string]any)["id"] != id || f.funds.execCalls.Load() != 1 {
		t.Fatal("request replay recreated the refund or executed funds")
	}
	decision := map[string]any{"decision": "approve", "reason": "Synthetic approval"}
	v, err = a06RefundCall(f, sellerToken, "decide_refund", id, decision)
	must(t, err)
	result := v.(map[string]any)
	if result["status"] != "completed" || result["amount_quota"] != int64(100) || result["quantity"] != int64(2) || result["completed_at"].(int64) <= 0 {
		t.Fatal(result)
	}
	_, err = a06RefundCall(f, sellerToken, "decide_refund", id, decision)
	must(t, err)
	if f.funds.execCalls.Load() != 2 || f.order(t).State != Refunded || !f.order(t).Delivered {
		t.Fatal("approval replay repeated funds or retracted delivery")
	}
	var stock Variant
	variantID := f.order(t).VariantID
	must(t, f.repo.Within(context.Background(), f.shop, func(tx Tx) error { return tx.Get("variants", variantID, &stock) }))
	if stock.Stock != 8 || stock.Reserved != 0 || stock.Claimed != 2 {
		t.Fatalf("financial refund restocked delivered inventory: %+v", stock)
	}
	view, err := a06RefundCall(f, buyerToken, "refunds", "", nil)
	must(t, err)
	b, err := json.Marshal(view)
	must(t, err)
	if strings.Contains(string(b), "input_hash") || strings.Contains(string(b), "synthetic:") || strings.Contains(string(b), "synthetic delivery") {
		t.Fatal("private command, receipt or delivery leaked")
	}
}

func TestA06LegacyRefundUnknownReadsOnlyQueryAndExplicitApprovalUsesSameKey(t *testing.T) {
	f := a06RefundSetup(t)
	id := a06RefundOpen(t, f)
	f.funds.execute = func(MoneyCommand) (Receipt, error) { return Receipt{}, ErrPending }
	decision := map[string]any{"decision": "approve"}
	_, err := a06RefundCall(f, sellerToken, "decide_refund", id, decision)
	wantError(t, err, ErrPending)
	before, calls := f.image(t), f.funds.execCalls.Load()
	f.funds.query = func(c MoneyCommand) (Receipt, error) {
		r := a06StateReceipt(c)
		r.CommandHash = ID("invalid acknowledgment")
		return r, nil
	}
	f.s, err = New(&a06RefundIndex{memoryRepo: f.repo, shop: f.shop}, f.auth, f.funds)
	must(t, err)
	original := f.operation(t, "refund").Command
	f.funds.execute = func(c MoneyCommand) (Receipt, error) {
		if ID(c) != ID(original) || c.Key != original.Key {
			t.Error("explicit retry changed the original refund command")
		}
		return Receipt{}, ErrPending
	}
	_, err = a06RefundCall(f, sellerToken, "decide_refund", id, decision)
	wantError(t, err, ErrPending)
	calls++ // Explicitly repeating the original approval may resume that key.
	v, err := a06RefundCall(f, buyerToken, "refunds", "", nil)
	must(t, err)
	if f.image(t) != before || f.funds.execCalls.Load() != calls || v.(map[string]any)["refunds"].([]map[string]any)[0]["status"] != "reconciliation_required" {
		t.Fatal("UNKNOWN changed state or retried a financial write")
	}
	f.funds.query = nil
	v, err = a06RefundCall(f, buyerToken, "refunds", "", nil)
	must(t, err)
	if f.order(t).State != Refunded || f.funds.execCalls.Load() != calls || v.(map[string]any)["refunded_quota"] != int64(100) {
		t.Fatal("original receipt did not converge")
	}
}

func TestA06LegacyRefundRejectsAuthorityAndChangedRequestsWithoutEffects(t *testing.T) {
	f := a06RefundSetup(t)
	before := f.image(t)
	for _, token := range []string{sellerToken, otherToken} {
		_, err := a06RefundCall(f, token, "request_refund", "", a06RefundRequest())
		wantError(t, err, ErrForbidden)
	}
	if f.image(t) != before {
		t.Fatal("unauthorized request changed state")
	}
	id := a06RefundOpen(t, f)
	before = f.image(t)
	for _, token := range []string{buyerToken, otherToken} {
		_, err := a06RefundCall(f, token, "decide_refund", id, map[string]any{"decision": "approve"})
		wantError(t, err, ErrForbidden)
	}
	changed := a06RefundRequest()
	changed["reason"] = "changed"
	_, err := a06RefundCall(f, buyerToken, "request_refund", "", changed)
	wantError(t, err, ErrConflict)
	_, err = a06RefundCall(f, sellerToken, "decide_refund", ID("another-refund"), map[string]any{"decision": "approve"})
	wantError(t, err, ErrNotFound)
	if f.image(t) != before || f.funds.execCalls.Load() != 1 {
		t.Fatal("unauthorized decision or changed replay changed state")
	}
}

func TestA06LegacyRefundUnsupportedModesAndUnprovenPaymentStayUnchanged(t *testing.T) {
	for _, tc := range []struct {
		edit func(map[string]any)
		want error
	}{
		{func(v map[string]any) { v["mode"] = "quantity"; v["quantity"] = 1 }, ErrUnavailable},
		{func(v map[string]any) { v["mode"] = "amount"; v["amount_quota"] = 50 }, ErrUnavailable},
		{func(v map[string]any) { v["amount_minor"] = 1 }, ErrInvalid},
		{func(v map[string]any) { v["stock_ids"] = []string{"stock"} }, ErrInvalid},
		{func(v map[string]any) { v["mode"] = "unknown" }, ErrInvalid},
	} {
		f := a06RefundSetup(t)
		before := f.image(t)
		in := a06RefundRequest()
		tc.edit(in)
		_, err := a06RefundCall(f, buyerToken, "request_refund", "", in)
		wantError(t, err, tc.want)
		if f.image(t) != before || f.funds.execCalls.Load() != 1 {
			t.Fatal("unsupported refund changed state")
		}
	}
	f := a06RefundSetup(t)
	pay := f.operation(t, "pay")
	pay.Receipt.CommandHash = ID("forged")
	must(t, f.repo.Within(context.Background(), f.shop, func(tx Tx) error { return tx.Put("money_operations", pay.Command.Key, pay) }))
	before := f.image(t)
	_, err := a06RefundCall(f, buyerToken, "request_refund", "", a06RefundRequest())
	wantError(t, err, ErrConflict)
	if f.image(t) != before {
		t.Fatal("unproven payment produced a refund")
	}
}

func TestA06LegacyExplicitSameApprovalResumesAfterSellerLiquidityRecovers(t *testing.T) {
	f := a06RefundSetup(t)
	_, err := f.s.FinalizeDelivery(context.Background(), f.shop, f.id)
	must(t, err)
	id := a06RefundOpen(t, f)
	f.funds.execute = func(MoneyCommand) (Receipt, error) { return Receipt{}, ErrPending }
	decision := map[string]any{"decision": "approve", "reason": "Same original approval"}
	_, err = a06RefundCall(f, sellerToken, "decide_refund", id, decision)
	wantError(t, err, ErrPending)
	original := f.operation(t, "refund").Command
	f.funds.query = func(MoneyCommand) (Receipt, error) { return Receipt{}, ErrPending }
	f.funds.execute = func(c MoneyCommand) (Receipt, error) {
		if c.Key != original.Key || ID(c) != ID(original) {
			t.Fatal("liquidity recovery selected a new refund")
		}
		return a06StateReceipt(c), nil
	}
	v, err := a06RefundCall(f, sellerToken, "decide_refund", id, decision)
	must(t, err)
	if v.(map[string]any)["status"] != "completed" || f.funds.readCalls.Load() != 1 || f.funds.execCalls.Load() != 3 {
		t.Fatal("explicit approval did not recover the same pending operation")
	}
}

func TestA06LegacyRefundDecisionRaceAndCommitFailure(t *testing.T) {
	f := a06RefundSetup(t)
	f.s.repo.(*a06RefundIndex).failWrites.Store(true)
	_, err := a06RefundCall(f, buyerToken, "request_refund", "", a06RefundRequest())
	if err == nil || f.order(t).RefundState != "" || f.order(t).LegacyRefund != nil {
		t.Fatal("failed request commit left refund metadata")
	}
	id := a06RefundOpen(t, f)
	var wg sync.WaitGroup
	errs := make(chan error, 2)
	for _, decision := range []string{"approve", "reject"} {
		wg.Add(1)
		go func(decision string) {
			defer wg.Done()
			_, err := a06RefundCall(f, sellerToken, "decide_refund", id, map[string]any{"decision": decision})
			errs <- err
		}(decision)
	}
	wg.Wait()
	close(errs)
	conflicts := 0
	for err := range errs {
		if errors.Is(err, ErrConflict) {
			conflicts++
		} else if err != nil {
			t.Fatal(err)
		}
	}
	o := f.order(t)
	if conflicts != 1 || o.LegacyRefund.Decision == "approve" && o.State != Refunded || o.LegacyRefund.Decision == "reject" && o.RefundState != "rejected" {
		t.Fatalf("decision race did not converge: %+v conflicts=%d", o, conflicts)
	}
}

func TestA06LegacyRefundLostLocalCommitKeepsCompletionAtomic(t *testing.T) {
	f := a06RefundSetup(t)
	id := a06RefundOpen(t, f)
	f.funds.execute = func(c MoneyCommand) (Receipt, error) {
		f.repo.failCommit.Store(true)
		return a06StateReceipt(c), nil
	}
	_, err := a06RefundCall(f, sellerToken, "decide_refund", id, map[string]any{"decision": "approve"})
	wantError(t, err, ErrPending)
	o := f.order(t)
	if o.State != RefundPending || o.LegacyRefund.CompletedAt != 0 || o.RefundRef != "" || f.operation(t, "refund").Done {
		t.Fatal("failed local commit partially completed the refund")
	}
	calls := f.funds.execCalls.Load()
	_, err = a06RefundCall(f, buyerToken, "refunds", "", nil)
	must(t, err)
	o = f.order(t)
	if o.State != Refunded || o.LegacyRefund.CompletedAt <= 0 || !f.operation(t, "refund").Done || f.funds.execCalls.Load() != calls {
		t.Fatal("saved receipt recovery did not atomically finish original refund")
	}
}

func TestA06LegacyPermanentRefundRejectionIsTerminalAndPreservesFundsState(t *testing.T) {
	f := a06RefundSetup(t)
	id := a06RefundOpen(t, f)
	f.funds.execute = func(c MoneyCommand) (Receipt, error) {
		r := a06StateReceipt(c)
		r.Outcome = "rejected"
		return r, nil
	}
	_, err := a06RefundCall(f, sellerToken, "decide_refund", id, map[string]any{"decision": "approve"})
	wantError(t, err, ErrConflict)
	o := f.order(t)
	if o.State != Paid || o.RefundState != "rejected" || o.LegacyRefund.CompletedAt != 0 || o.RefundRef != "" {
		t.Fatal("permanent rejection was shown as pending or refunded")
	}
	view, err := a06RefundCall(f, buyerToken, "refunds", "", nil)
	must(t, err)
	if view.(map[string]any)["refunds"].([]map[string]any)[0]["status"] != "rejected" || f.funds.readCalls.Load() != 0 {
		t.Fatal("a known rejection was queried as UNKNOWN")
	}
}

type a06RefundDisabledBuyer struct{ *authority }

func (a a06RefundDisabledBuyer) Check(ctx context.Context, token string, account Account, permission Permission) (int64, error) {
	if token == buyerToken {
		return 0, ErrForbidden
	}
	return a.authority.Check(ctx, token, account, permission)
}

func TestA06LegacyRefundSellerCanReturnAcceptedOrderAfterBuyerDisabled(t *testing.T) {
	f := a06RefundSetup(t)
	id := a06RefundOpen(t, f)
	f.s.auth = a06RefundDisabledBuyer{f.auth}
	v, err := a06RefundCall(f, sellerToken, "decide_refund", id, map[string]any{"decision": "approve"})
	must(t, err)
	if v.(map[string]any)["status"] != "completed" || f.order(t).Buyer != (Account{"personal", 2}) {
		t.Fatal("buyer deactivation prevented or redirected the accepted refund")
	}
}
