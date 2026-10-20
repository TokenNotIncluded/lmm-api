// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package toolmarket

import (
	"context"
	"errors"
	"testing"
)

type cancellableFunds struct {
	*fakeFunds
	releases   int
	dispatched bool
}

func (f *cancellableFunds) ReleaseUndispatched(_ context.Context, _ string, p Principal, c Charge) error {
	f.mu.Lock()
	defer f.mu.Unlock()
	if p != alice || f.reserves[c.ID] != c || f.dispatched {
		return ErrPending
	}
	f.releases++
	return nil
}
func TestCancelOriginalReservedCallOnceWithoutExternalRetry(t *testing.T) {
	mock := newMock(t)
	m := testModule(t, mock)
	funds := &cancellableFunds{fakeFunds: newFunds()}
	funds.failReserve = true
	m.funds = funds
	_, installation, _ := prepared(t, m, mock, "none", "fixed")
	input := invocation(t, m, alice, installation.ID, "cancel-original")
	if _, err := m.execute(context.Background(), alice, "alice", input); !errors.Is(err, ErrPending) {
		t.Fatal(err)
	}
	// The controlled reserve kept an original hold but lost its acknowledgement.
	// CorePG independently verifies actual hold release; this tests module state.
	if _, err := m.cancelExecution(context.Background(), alice, "alice", input.IdempotencyKey); err != nil {
		t.Fatal(err)
	}
	if _, err := m.cancelExecution(context.Background(), alice, "alice", input.IdempotencyKey); err != nil {
		t.Fatal(err)
	}
	if _, err := m.execute(context.Background(), alice, "alice", input); !errors.Is(err, ErrPending) {
		t.Fatal("cancelled call retried", err)
	}
	if funds.releases != 1 || mock.calls.Load() != 0 {
		t.Fatal("duplicate release or external retry", funds.releases, mock.calls.Load())
	}
}
func TestCancelCannotTurnDispatchUnknownIntoRefund(t *testing.T) {
	mock := newMock(t)
	m := testModule(t, mock)
	funds := &cancellableFunds{fakeFunds: newFunds(), dispatched: true}
	funds.failReserve = true
	m.funds = funds
	_, installation, _ := prepared(t, m, mock, "none", "fixed")
	input := invocation(t, m, alice, installation.ID, "uncertain")
	if _, err := m.execute(context.Background(), alice, "alice", input); !errors.Is(err, ErrPending) {
		t.Fatal(err)
	}
	if _, err := m.cancelExecution(context.Background(), alice, "alice", input.IdempotencyKey); !errors.Is(err, ErrPending) {
		t.Fatal("refunded dispatch uncertainty", err)
	}
	if funds.releases != 0 || mock.calls.Load() != 0 {
		t.Fatal("uncertain operation changed")
	}
	id := digest([]string{"toolmarket/v1", alice.key(), input.IdempotencyKey})
	if err := m.store.view(func(d database) error {
		if d.Executions[id].State != "started" {
			return ErrConflict
		}
		return nil
	}); err != nil {
		t.Fatal(err)
	}
}
