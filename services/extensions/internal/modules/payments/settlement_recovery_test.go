package payments

import (
	"context"
	"testing"
)

// dispatchRecoveryCore can lose access to registration independently of the
// read-only receipt endpoint. It never contacts a provider or a real Core.
type dispatchRecoveryCore struct {
	Core
	lookupErr         error
	registrationCalls int
}

func (c *dispatchRecoveryCore) BeginRefundDispatch(context.Context, int64, string, string) (Receipt, error) {
	c.registrationCalls++
	return Receipt{}, ErrUnavailable
}

func (c *dispatchRecoveryCore) Receipt(ctx context.Context, intent int64, key string) (Receipt, error) {
	if c.lookupErr != nil {
		return Receipt{}, c.lookupErr
	}
	return c.Core.Receipt(ctx, intent, key)
}

func TestRefundDispatchRecoveryReadsOriginalReceiptBeforeRegistration(t *testing.T) {
	for _, tc := range []struct {
		name             string
		lookupErr        error
		wantRegistration int
		wantProvider     int64
	}{
		{name: "committed registration", wantProvider: 1},
		{name: "unknown receipt", lookupErr: ErrUnavailable},
		{name: "conflicting receipt", lookupErr: ErrConflict},
		{name: "missing receipt", lookupErr: ErrNotFound, wantRegistration: 1},
	} {
		t.Run(tc.name, func(t *testing.T) {
			h := setup(t)
			o := h.paid()
			r, err := h.s.RequestRefund(context.Background(), "user", o.ID, "dispatch_recovery_0001", 100)
			if err != nil {
				t.Fatal(err)
			}
			h.c.lose["dispatch"] = true
			h.work()
			if h.a.refundCalls.Load() != 0 || h.c.count("dispatch") != 1 {
				t.Fatal("fixture must lose registration ACK before provider dispatch")
			}
			core := &dispatchRecoveryCore{Core: h.c, lookupErr: tc.lookupErr}
			h.s.core = core
			h.advance()
			h.work()
			if core.registrationCalls != tc.wantRegistration || h.a.refundCalls.Load() != tc.wantProvider {
				t.Fatalf("registration calls=%d provider calls=%d", core.registrationCalls, h.a.refundCalls.Load())
			}
			last := h.get(o.ID)
			if last.Refunds[r.ID].HoldReceipt != r.HoldReceipt || h.c.count("hold") != 1 || h.c.count("release") != 0 || h.c.count("commit") != 0 {
				t.Fatal("registration recovery changed funds or the original hold")
			}
			job := last.Jobs[stableKey("submit", r.ID)]
			if tc.wantProvider == 1 {
				if job.Kind != "refund_submit" || job.State != "done" || last.Refunds[r.ID].DispatchReceipt == "" {
					t.Fatal("original registration receipt did not recover the send-once job")
				}
				h.advance()
				h.work()
				if h.a.refundCalls.Load() != 1 {
					t.Fatal("recovered refund was dispatched twice")
				}
			} else if job.Kind != "refund_dispatch" || job.State == "done" || last.Refunds[r.ID].DispatchReceipt != "" {
				t.Fatal("unresolved registration was treated as accepted")
			}
		})
	}
}

type failFirstDispatchCore struct {
	Core
	calls int
	keys  []string
}

func (c *failFirstDispatchCore) BeginRefundDispatch(ctx context.Context, intent int64, key, hold string) (Receipt, error) {
	c.calls++
	c.keys = append(c.keys, key)
	if c.calls == 1 {
		return Receipt{}, ErrUnavailable // This synthetic failure occurs before commit.
	}
	return c.Core.BeginRefundDispatch(ctx, intent, key, hold)
}

type checkDispatchPersistedAdapter struct {
	Adapter
	store    Store
	refundID string
	t        *testing.T
}

func (a *checkDispatchPersistedAdapter) Refund(ctx context.Context, o Order, r Refund, key string) (RefundResult, error) {
	stored, err := a.store.Get(ctx, o.ID)
	if err != nil {
		return RefundResult{}, err
	}
	job := stored.Jobs[key]
	if job.Kind != "refund_submit" || job.State != "running" || stored.Refunds[a.refundID].DispatchReceipt == "" || stored.Refunds[a.refundID].Dispatch != DispatchUnknown {
		a.t.Error("registration and send-once claim must be persisted before provider call")
		return RefundResult{}, ErrConflict
	}
	return a.Adapter.Refund(ctx, o, r, key)
}

func TestRefundDispatchMissingReceiptRegistersOriginalKeyOnce(t *testing.T) {
	h := setup(t)
	o := h.paid()
	r, err := h.s.RequestRefund(context.Background(), "user", o.ID, "dispatch_missing_0001", 100)
	if err != nil {
		t.Fatal(err)
	}
	core := &failFirstDispatchCore{Core: h.c}
	h.s.core = core
	h.s.adapters[o.ChannelID] = &checkDispatchPersistedAdapter{Adapter: h.a, store: h.m, refundID: r.ID, t: t}
	h.work()
	key := stableKey("submit", r.ID)
	if core.calls != 1 || h.a.refundCalls.Load() != 0 || h.get(o.ID).Jobs[key].State != "pending" {
		t.Fatal("failed first registration must remain pending before provider IO")
	}
	h.advance()
	h.work()
	last := h.get(o.ID)
	if core.calls != 2 || core.keys[0] != key || core.keys[1] != key || h.c.count("lookup") != 1 || h.c.count("dispatch") != 1 {
		t.Fatal("missing receipt must retry original registration key exactly once")
	}
	if last.Jobs[key].Kind != "refund_submit" || last.Jobs[key].State != "done" || last.Refunds[r.ID].DispatchReceipt == "" || last.Refunds[r.ID].HoldReceipt != r.HoldReceipt || h.a.refundCalls.Load() != 1 {
		t.Fatal("successful registration did not persist and dispatch exactly once")
	}
	h.advance()
	h.work()
	if h.a.refundCalls.Load() != 1 || core.calls != 2 || h.c.count("hold") != 1 || h.c.count("release") != 0 || h.c.count("commit") != 0 {
		t.Fatal("recovery repeated dispatch or changed the hold")
	}
}

func withoutPaymentAdapters(t *testing.T, h *harness) {
	t.Helper()
	s, err := New(h.m, h.c)
	if err != nil {
		t.Fatal(err)
	}
	s.now = h.s.now
	h.s = s
}

func TestSettlementRecoversCommittedCreditWithoutAdapter(t *testing.T) {
	h := setup(t)
	o := h.create("lost_credit_response_0001")
	h.work()
	h.c.lose["credit"] = true
	h.send(h.event(o, "evt_lost_credit", Paid, 1000))
	h.work()
	if h.get(o.ID).Credited() || h.c.count("credit") != 1 {
		t.Fatal("fixture must lose the response after the first core credit")
	}
	withoutPaymentAdapters(t, h)
	h.advance()
	h.work()
	if !h.get(o.ID).Credited() || h.c.count("credit") != 1 || h.c.count("lookup") != 1 {
		t.Fatal("core receipt recovery must not need a provider adapter or repeat credit")
	}
	if h.a.checkoutCalls.Load() != 1 || h.a.refundCalls.Load() != 0 {
		t.Fatal("receipt recovery performed provider IO")
	}
}

func TestSettlementRecoversLostHoldWithoutAdapter(t *testing.T) {
	h := setup(t)
	o := h.paid()
	h.c.lose["hold"] = true
	r, err := h.s.RequestRefund(context.Background(), "user", o.ID, "lost_hold_response_0001", 100)
	if err != nil || r.State != "hold_pending" {
		t.Fatalf("fixture hold: state=%s err=%v", r.State, err)
	}
	withoutPaymentAdapters(t, h)
	h.work()
	r = h.get(o.ID).Refunds[r.ID]
	if r.State != "held" || r.HoldReceipt == "" || h.c.count("hold") != 1 {
		t.Fatal("lost hold receipt did not recover independently of provider configuration")
	}
	h.work()
	j := h.get(o.ID).Jobs[stableKey("submit", r.ID)]
	if j.Attempts != 0 || j.State != "pending" || h.a.refundCalls.Load() != 0 {
		t.Fatal("missing provider must still prevent the first external refund attempt")
	}
}

func TestSettlementUsesAcceptedProofWithoutAdapter(t *testing.T) {
	h := setup(t)
	o := h.create("accepted_credit_0001")
	h.work()
	h.send(h.event(o, "evt_accepted_credit", Paid, 1000))
	if err := h.s.Drain(context.Background(), 100); err != nil {
		t.Fatal(err)
	}
	withoutPaymentAdapters(t, h)
	h.work()
	if !h.get(o.ID).Credited() || h.c.count("credit") != 1 {
		t.Fatal("a queued core-accepted proof was blocked by missing Go adapter configuration")
	}
}

func TestSettlementStillRejectsUnacceptedEvidenceWithoutAdapter(t *testing.T) {
	h := setup(t)
	o := h.create("unaccepted_credit_0001")
	h.work()
	e := h.event(o, "evt_unsigned_credit", Paid, 1000)
	e.Evidence.Source, e.Evidence.Signature = "api", nil
	h.send(e)
	if err := h.s.Drain(context.Background(), 100); err != nil {
		t.Fatal(err)
	}
	withoutPaymentAdapters(t, h)
	h.work()
	if h.get(o.ID).Credited() || h.c.count("credit") != 0 {
		t.Fatal("provider observation was treated as an accepted core proof")
	}
	for _, j := range h.get(o.ID).Jobs {
		if j.Kind == "credit" && (j.State != "review" || j.ErrorCode != ErrEvidence.Error()) {
			t.Fatal("unsigned evidence must remain in review")
		}
	}
}
