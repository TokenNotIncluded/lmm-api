// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package toolmarket

import (
	"context"
	"errors"
	"testing"
)

func TestLegacyPublishedQuoteReadRetainsOriginalFullDigests(t *testing.T) {
	m, _, mock := webFixture(t)
	in := webInput(mock.server.URL+"/mcp", "public")
	in.Tools[0].PriceQuota = 18
	s := webSaved(t, m, "1", in)
	webPublish(t, m, "1", s)
	var snapshot webSnapshot
	if err := m.store.view(func(d database) error { snapshot = d.Web.Versions[s.DraftVersionID]; return nil }); err != nil {
		t.Fatal(err)
	}
	expected, err := legacyVersionCreditQuotes(s.ID, snapshot)
	if err != nil {
		t.Fatal(err)
	}
	got, err := m.LegacyReadToolQuote(context.Background(), "2", "2", s.ID, s.DraftVersionID, snapshot.Tools[0].ToolID)
	if err != nil || len(expected) != 1 || digest(got) != digest(expected[0]) {
		t.Fatal("read changed registered quote", err)
	}
	if got.RemoteDigest == digest(snapshot.Tools[0].InputSchema) {
		t.Fatal("full remote digest replaced by input schema digest")
	}
	if _, err = m.LegacyReadToolQuote(context.Background(), "2", "2", "wrong-service", s.DraftVersionID, snapshot.Tools[0].ToolID); !errors.Is(err, ErrConflict) {
		t.Fatal("service/version mismatch accepted", err)
	}
	if err := m.store.update(func(d *database) error {
		v := d.Web.Versions[s.DraftVersionID]
		v.ValidationDigest = ""
		d.Web.Versions[s.DraftVersionID] = v
		return nil
	}); err != nil {
		t.Fatal(err)
	}
	if _, err = m.LegacyReadToolQuote(context.Background(), "1", "1", s.ID, s.DraftVersionID, snapshot.Tools[0].ToolID); !errors.Is(err, ErrConflict) {
		t.Fatal("unvalidated snapshot accepted", err)
	}
}
