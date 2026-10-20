// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package toolmarket

import (
	"context"
	"errors"
	"testing"
)

func TestLegacyRequestKeyInspectionNeverCreatesOrDispatches(t *testing.T) {
	m, port, r := executionFixture(t)
	a := &executionAuthority{executionCredits: *port}
	m.authority = a
	r.Input.ClientID = "builtin-assistant"
	r.ID = digest([]any{int64(1), r.Input.ClientID, r.Input.RequestKey})
	r.Core.ID, r.Core.ClientID = r.ID, r.Input.ClientID
	a.r = r.Core
	if err := m.store.update(func(d *database) error { d.LegacyExecutions = map[string]LegacyExecution{r.ID: r}; return nil }); err != nil {
		t.Fatal(err)
	}
	got, err := m.LegacyGetExecutionByRequestKey(context.Background(), "1", "1", r.Input.ClientID, r.Input.RequestKey)
	if !errors.Is(err, ErrPending) || got.ID != r.ID || a.reserves != 0 || a.finishes != 0 || a.releases != 0 {
		t.Fatal("inspection changed original operation", got.ID, err)
	}
	if _, err := m.LegacyGetExecutionByRequestKey(context.Background(), "1", "1", r.Input.ClientID, "missing"); !errors.Is(err, ErrMissing) {
		t.Fatal("missing operation recreated", err)
	}
}
