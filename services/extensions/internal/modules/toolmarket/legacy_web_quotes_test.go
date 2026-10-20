// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package toolmarket

import (
	"errors"
	"testing"
)

func TestWebPublicationWaitsForCoreQuoteAndAdminApproval(t *testing.T) {
	m, authority, mock := webFixture(t)
	core := &quoteCoreFixture{webTestAuthority: authority, fail: ErrUnavailable}
	m.authority = core
	service := webSaved(t, m, "1", webInput(mock.server.URL+"/mcp", "public"))
	body := map[string]any{"version_id": service.DraftVersionID}
	if _, err := webRequest(t, m, "1", "submit", service.ID, body); !errors.Is(err, ErrUnavailable) {
		t.Fatal("publication did not wait for Core quote", err)
	}
	if len(webCatalogue(t, m, "", "")) != 0 || len(core.approvals) != 0 {
		t.Fatal("unregistered quote became visible or approved")
	}
	core.fail = nil
	if _, err := webRequest(t, m, "1", "submit", service.ID, body); err != nil {
		t.Fatal(err)
	}
	core.fail = ErrUnavailable
	if _, err := webRequest(t, m, "3", "review", service.ID, map[string]any{"version_id": service.DraftVersionID, "approve": true, "note": "fixture"}); !errors.Is(err, ErrUnavailable) {
		t.Fatal("review did not wait for Core approval", err)
	}
	if len(webCatalogue(t, m, "", "")) != 0 {
		t.Fatal("unacknowledged approval became public")
	}
	core.fail = nil
	if _, err := webRequest(t, m, "3", "review", service.ID, map[string]any{"version_id": service.DraftVersionID, "approve": true, "note": "fixture"}); err != nil {
		t.Fatal(err)
	}
	if len(webCatalogue(t, m, "", "")) != 1 || len(core.approvals) != 2 || core.approvals[1].Visibility != "public" || !core.approvals[1].Approved {
		t.Fatal("registered and admin approved snapshot not published")
	}
}
