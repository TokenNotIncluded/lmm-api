// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package toolmarket

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"testing"
)

type resultRecoveryFunds struct {
	*fakeFunds
	commits    int
	hash       string
	credential string
}

func (f *resultRecoveryFunds) CommitResult(_ context.Context, credential string, p Principal, c Charge, hash string) error {
	if p != alice || c.ID != "saved" {
		return ErrDenied
	}
	f.commits++
	f.hash, f.credential = hash, credential
	return nil
}
func TestNativeBackgroundRecoveryUsesOnlyRetainedOriginalResult(t *testing.T) {
	mock := newMock(t)
	m := testModule(t, mock)
	funds := &resultRecoveryFunds{fakeFunds: newFunds()}
	m.funds = funds
	result := json.RawMessage(`{"content":[{"type":"text","text":"retained"}]}`)
	if err := m.store.update(func(d *database) error {
		d.Executions["saved"] = Execution{Owner: alice, State: "settlement_pending", Charge: Charge{ID: "saved"}, Result: result}
		d.Executions["unknown"] = Execution{Owner: alice, State: "dispatched", Charge: Charge{ID: "unknown"}}
		return nil
	}); err != nil {
		t.Fatal(err)
	}
	completed, err := m.RecoverSettlements(context.Background(), 20)
	if err != nil || completed != 1 || funds.commits != 1 || funds.credential != "" || mock.calls.Load() != 0 {
		t.Fatal("recovery redispatched or required login", completed, err)
	}
	hash := sha256.Sum256(result)
	if funds.hash != hex.EncodeToString(hash[:]) {
		t.Fatal("original persisted result hash changed")
	}
	completed, err = m.RecoverSettlements(context.Background(), 20)
	if err != nil || completed != 0 || funds.commits != 1 {
		t.Fatal("terminal recovery repeated", completed, err)
	}
	if err := m.store.view(func(d database) error {
		if d.Executions["unknown"].State != "dispatched" {
			return ErrConflict
		}
		return nil
	}); err != nil {
		t.Fatal("unknown effect changed", err)
	}
}
