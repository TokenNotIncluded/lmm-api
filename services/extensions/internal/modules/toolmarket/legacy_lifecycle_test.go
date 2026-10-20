// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package toolmarket

import (
	"context"
	"encoding/json"
	"errors"
	"strconv"
	"testing"
	"time"
)

type lifecycleCoreFixture struct {
	*webTestAuthority
	owner   int64
	state   LegacyServiceLifecycleReceipt
	saved   map[string]LegacyServiceLifecycleReceipt
	calls   int
	fail    error
	corrupt string
	hook    func(LegacyServiceLifecycleReceipt)
}

func (a *lifecycleCoreFixture) SetLegacyServiceLifecycle(_ context.Context, credential string, in LegacyServiceLifecycleRequest) (LegacyServiceLifecycleReceipt, error) {
	a.calls++
	if a.fail != nil {
		return LegacyServiceLifecycleReceipt{}, a.fail
	}
	actor, _ := strconv.ParseInt(credential, 10, 64)
	if old, ok := a.saved[in.OperationID]; ok {
		if old.LegacyServiceLifecycleRequest != in || old.Revision != a.state.Revision {
			return LegacyServiceLifecycleReceipt{}, ErrConflict
		}
		return old, nil
	}
	if a.state.Deleted && !in.Deleted {
		return LegacyServiceLifecycleReceipt{}, ErrConflict
	}
	if actor == a.owner && a.state.Suspended && !in.Paused && !in.Deleted {
		return LegacyServiceLifecycleReceipt{}, ErrDenied
	}
	suspended := a.state.Suspended
	if actor != a.owner && !in.Deleted {
		suspended = in.Paused
	}
	out := LegacyServiceLifecycleReceipt{LegacyServiceLifecycleRequest: in, Suspended: suspended, Revision: a.state.Revision + 1, UpdatedBy: actor, UpdatedAt: 100}
	a.state = out
	a.saved[in.OperationID] = out
	switch a.corrupt {
	case "actor":
		out.UpdatedBy = 2
	case "revision":
		out.Revision = 0
	case "echo":
		out.Paused = !out.Paused
	case "suspension":
		out.Suspended = false
	}
	if a.hook != nil {
		a.hook(out)
	}
	return out, nil
}
func lifecycleModule(t *testing.T) (*Module, *lifecycleCoreFixture) {
	t.Helper()
	a := &lifecycleCoreFixture{webTestAuthority: &webTestAuthority{}, owner: 1, state: LegacyServiceLifecycleReceipt{Revision: 1}, saved: map[string]LegacyServiceLifecycleReceipt{}}
	s := &Store{data: legacyFixtureDB()}
	m, e := New(s, a, Config{WebOnly: true})
	if e != nil {
		t.Fatal(e)
	}
	m.now = func() time.Time { return time.Unix(100, 0) }
	t.Cleanup(func() { s.Close() })
	return m, a
}
func applyLifecycleTest(t *testing.T, m *Module, r LegacyServiceLifecycleReceipt) {
	t.Helper()
	if e := m.store.update(func(d *database) error {
		s := d.Web.Services[r.ServiceID]
		if e := legacyValidateLifecycleAck(s.CoreLifecycleRevision, s.Status, r); e != nil {
			return e
		}
		s.CoreLifecycleRevision = r.Revision
		s.Status = legacyLifecycleStatus(r)
		d.Web.Services[r.ServiceID] = s
		return nil
	}); e != nil {
		t.Fatal(e)
	}
}
func TestLegacyLifecycleOwnerPauseUnpauseAndStaleOperation(t *testing.T) {
	m, a := lifecycleModule(t)
	ctx := context.Background()
	paused, e := m.legacySetServiceLifecycle(ctx, "1", "1", "pause-1", "service", true, false)
	if e != nil || paused.Suspended || paused.Revision != 2 {
		t.Fatalf("%+v %v", paused, e)
	}
	if m.store.data.Web.Services["service"].Status != "published" {
		t.Fatal("helper changed local state before parent ACK transaction")
	}
	applyLifecycleTest(t, m, paused)
	unpaused, e := m.legacySetServiceLifecycle(ctx, "1", "1", "resume", "service", false, false)
	if e != nil || unpaused.Revision != 3 {
		t.Fatalf("%+v %v", unpaused, e)
	}
	applyLifecycleTest(t, m, unpaused)
	if _, e = m.legacySetServiceLifecycle(ctx, "1", "1", "pause-1", "service", true, false); !errors.Is(e, ErrConflict) {
		t.Fatal("old operation revived later state", e)
	}
	next, e := m.legacySetServiceLifecycle(ctx, "1", "1", "pause-2", "service", true, false)
	if e != nil || next.Revision != 4 {
		t.Fatalf("new pause %+v %v", next, e)
	}
	applyLifecycleTest(t, m, next)
	again, e := m.legacySetServiceLifecycle(ctx, "1", "1", "pause-2", "service", true, false)
	if e != nil || again != next {
		t.Fatalf("idempotent replay %+v %v", again, e)
	}
	if a.calls != 5 {
		t.Fatal(a.calls)
	}
}
func TestLegacyLifecycleSuspensionAndPermanentDeletion(t *testing.T) {
	m, a := lifecycleModule(t)
	ctx := context.Background()
	suspended, e := m.legacySetServiceLifecycle(ctx, "3", "3", "suspend", "service", true, false)
	if e != nil || !suspended.Suspended {
		t.Fatalf("%+v %v", suspended, e)
	}
	applyLifecycleTest(t, m, suspended)
	before := a.calls
	if _, e = m.legacySetServiceLifecycle(ctx, "1", "1", "resume", "service", false, false); !errors.Is(e, ErrDenied) || a.calls != before {
		t.Fatal("owner cleared admin suspension", e)
	}
	deleted, e := m.legacySetServiceLifecycle(ctx, "1", "1", "delete", "service", false, true)
	if e != nil || !deleted.Deleted {
		t.Fatalf("owner delete suspended %+v %v", deleted, e)
	}
	applyLifecycleTest(t, m, deleted)
	if _, e = m.legacySetServiceLifecycle(ctx, "3", "3", "revive", "service", false, false); !errors.Is(e, ErrConflict) {
		t.Fatal("deleted service revived", e)
	}
}
func TestLegacyLifecycleAdminCanUnsuspendOtherOwner(t *testing.T) {
	m, _ := lifecycleModule(t)
	ctx := context.Background()
	suspended, e := m.legacySetServiceLifecycle(ctx, "3", "3", "suspend", "service", true, false)
	if e != nil {
		t.Fatal(e)
	}
	applyLifecycleTest(t, m, suspended)
	resumed, e := m.legacySetServiceLifecycle(ctx, "3", "3", "resume", "service", false, false)
	if e != nil || resumed.Suspended {
		t.Fatalf("%+v %v", resumed, e)
	}
	applyLifecycleTest(t, m, resumed)
	service := m.store.data.Web.Services["service"]
	service.OwnerID = 3
	service.Status = "suspended"
	m.store.data.Web.Services["service"] = service
	if _, e = m.legacySetServiceLifecycle(ctx, "3", "3", "self-resume", "service", false, false); !errors.Is(e, ErrDenied) {
		t.Fatal("admin owner cleared own suspension", e)
	}
}
func TestLegacyLifecycleRejectsForgedOrOutOfOrderAcknowledgement(t *testing.T) {
	for _, bad := range []string{"actor", "revision", "echo", "suspension", "late"} {
		t.Run(bad, func(t *testing.T) {
			m, a := lifecycleModule(t)
			a.corrupt = bad
			if bad == "late" {
				a.hook = func(r LegacyServiceLifecycleReceipt) {
					_ = m.store.update(func(d *database) error {
						s := d.Web.Services["service"]
						s.CoreLifecycleRevision = r.Revision + 1
						s.Status = "published"
						d.Web.Services["service"] = s
						return nil
					})
				}
			}
			if _, e := m.legacySetServiceLifecycle(context.Background(), "3", "3", "pause", "service", true, false); e == nil {
				t.Fatal("bad receipt accepted", bad)
			}
		})
	}
	r := LegacyServiceLifecycleReceipt{LegacyServiceLifecycleRequest: LegacyServiceLifecycleRequest{OperationID: "pause", ServiceID: "service", Paused: true}, Revision: 2, UpdatedBy: 1, UpdatedAt: 100}
	if e := legacyValidateLifecycleAck(3, "published", r); !errors.Is(e, ErrConflict) {
		t.Fatal(e)
	}
	if e := legacyValidateLifecycleAck(2, "published", r); !errors.Is(e, ErrConflict) {
		t.Fatal("same revision changed state", e)
	}
	if e := legacyValidateLifecycleAck(2, "paused", r); e != nil {
		t.Fatal(e)
	}
	r.Revision = 4
	if e := legacyValidateLifecycleAck(3, "deleted", r); !errors.Is(e, ErrConflict) {
		t.Fatal("permanent delete cleared", e)
	}
}
func TestLegacyLifecycleMissingCoreNeverBecomesFakeAck(t *testing.T) {
	m, a := lifecycleModule(t)
	a.fail = ErrMissing
	service := m.store.data.Web.Services["service"]
	service.Status = "draft"
	service.LiveVersionID = ""
	service.DraftVersionID = "version"
	m.store.data.Web.Services["service"] = service
	if _, e := m.legacySetServiceLifecycle(context.Background(), "1", "1", "delete", "service", false, true); !errors.Is(e, ErrMissing) {
		t.Fatal("unregistered draft got fake Core receipt", e)
	}
	if m.store.data.Web.Services["service"].Status != "draft" {
		t.Fatal("helper performed local fallback")
	}
	without, _, _ := webFixture(t)
	if _, e := without.legacySetServiceLifecycle(context.Background(), "1", "1", "pause", "service", true, false); !errors.Is(e, ErrUnavailable) {
		t.Fatal(e)
	}
}
func TestLegacyLifecycleWireRequiresExplicitStateAndRevision(t *testing.T) {
	for _, raw := range []string{`{"operation_id":"op","service_id":"service","deleted":false,"suspended":false,"revision":2,"updated_by":1,"updated_at":100}`, `{"operation_id":"op","service_id":"service","paused":false,"deleted":false,"suspended":null,"revision":2,"updated_by":1,"updated_at":100}`, `{"operation_id":"op","service_id":"service","paused":false,"deleted":false,"suspended":false,"updated_by":1,"updated_at":100}`} {
		var r LegacyServiceLifecycleReceipt
		if e := json.Unmarshal([]byte(raw), &r); e == nil {
			t.Fatal("implicit state or revision accepted", raw)
		}
	}
}

func TestLegacyLifecycleDraftDeleteRequiresRealTombstoneReceipt(t *testing.T) {
	m, _ := lifecycleModule(t)
	service := m.store.data.Web.Services["service"]
	service.Status = "draft"
	service.LiveVersionID = ""
	service.DraftVersionID = "version"
	m.store.data.Web.Services["service"] = service
	receipt, e := m.legacySetServiceLifecycle(context.Background(), "1", "1", "retire-draft", "service", false, true)
	if e != nil || !receipt.Deleted || receipt.Revision != 2 {
		t.Fatalf("%+v %v", receipt, e)
	}
	if m.store.data.Web.Services["service"].Status != "draft" {
		t.Fatal("helper applied local deletion before parent transaction")
	}
	applyLifecycleTest(t, m, receipt)
	repeat, e := m.legacySetServiceLifecycle(context.Background(), "1", "1", "retire-draft", "service", false, true)
	if e != nil || repeat != receipt {
		t.Fatalf("tombstone replay %+v %v", repeat, e)
	}
}
