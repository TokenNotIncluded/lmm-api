// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package toolmarket

import (
	"context"
	"errors"
	"testing"
	"time"
)

type webLifecycleAuthority struct {
	*webTestAuthority
	fail       error
	revision   int64
	operations []string
}

func (a *webLifecycleAuthority) SetLegacyServiceLifecycle(_ context.Context, _ string, in LegacyServiceLifecycleRequest) (LegacyServiceLifecycleReceipt, error) {
	a.operations = append(a.operations, in.OperationID)
	if a.fail != nil {
		return LegacyServiceLifecycleReceipt{}, a.fail
	}
	a.revision++
	return LegacyServiceLifecycleReceipt{LegacyServiceLifecycleRequest: in, Revision: a.revision, UpdatedBy: 1, UpdatedAt: 100}, nil
}
func TestWebLifecycleRequiresCoreACKAndRetainsTombstones(t *testing.T) {
	m, authority, mock := webFixture(t)
	m.now = func() time.Time { return time.Unix(100, 0) }
	service := webSaved(t, m, "1", webInput(mock.server.URL+"/mcp", "public"))
	webPublish(t, m, "1", service)
	core := &webLifecycleAuthority{webTestAuthority: authority, fail: ErrUnavailable, revision: 1}
	m.authority = core
	if _, err := webRequest(t, m, "1", "pause", service.ID, map[string]any{"paused": true}); !errors.Is(err, ErrUnavailable) {
		t.Fatal(err)
	}
	if len(webCatalogue(t, m, "", "")) != 1 {
		t.Fatal("failed Core pause changed local publication")
	}
	core.fail = nil
	for _, paused := range []bool{true, false, true} {
		if _, err := webRequest(t, m, "1", "pause", service.ID, map[string]any{"paused": paused}); err != nil {
			t.Fatal(err)
		}
	}
	if core.operations[1] == core.operations[3] {
		t.Fatal("later pause reused stale earlier operation")
	}
	if _, err := webRequest(t, m, "1", "delete", service.ID, nil); err != nil {
		t.Fatal(err)
	}
	if err := m.store.view(func(d database) error {
		s := d.Web.Services[service.ID]
		if s.Status != "deleted" || s.CoreLifecycleRevision != 5 {
			return ErrConflict
		}
		return nil
	}); err != nil {
		t.Fatal("Core tombstone not persisted atomically", err)
	}
}
func TestWebDraftDeleteDoesNotConvertCoreMissingToSuccess(t *testing.T) {
	m, authority, mock := webFixture(t)
	s := webSaved(t, m, "1", webInput(mock.server.URL+"/mcp", "private"))
	m.authority = &webLifecycleAuthority{webTestAuthority: authority, fail: ErrMissing}
	if _, err := webRequest(t, m, "1", "delete", s.ID, nil); !errors.Is(err, ErrMissing) {
		t.Fatal("missing Core tombstone accepted", err)
	}
	if err := m.store.view(func(d database) error {
		if d.Web.Services[s.ID].Status != "draft" {
			return ErrConflict
		}
		return nil
	}); err != nil {
		t.Fatal(err)
	}
}
