// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package toolmarket

import (
	"context"
	"errors"
	"testing"
)

type quoteAuthority struct {
	Authority
	registrations []QuoteRegistration
	registerErr   error
	token         string
	after         func()
}

func (a *quoteAuthority) RegisterToolQuote(_ context.Context, token string, q QuoteRegistration) error {
	a.token = token
	a.registrations = append(a.registrations, q)
	if a.after != nil {
		a.after()
	}
	return a.registerErr
}
func TestPublicationRequiresCoreQuoteAcknowledgementBeforeExposure(t *testing.T) {
	mock := newMock(t)
	m := testModule(t, mock)
	_, _, release := prepared(t, m, mock, "none", "fixed")
	if err := m.publish(alice, PublishInput{Release: release.ID, Published: false}); err != nil {
		t.Fatal(err)
	}
	auth := &quoteAuthority{Authority: m.authority, registerErr: ErrUnavailable}
	m.authority = auth
	publish := PublishInput{Release: release.ID, Published: true}
	if err := m.publishAuthorized(context.Background(), alice, "alice", publish); !errors.Is(err, ErrUnavailable) {
		t.Fatal(err)
	}
	if err := m.store.view(func(d database) error {
		if d.Releases[release.ID].Published {
			return ErrConflict
		}
		return nil
	}); err != nil {
		t.Fatal("exposed release without registered price", err)
	}
	if len(auth.registrations) != 1 || auth.token != "alice" {
		t.Fatal("publisher session not supplied")
	}
	q := auth.registrations[0]
	expected := quoteFor(release, release.Server.Tools[0], m.now())
	if q.ID != expected.ID || q.PriceHash != expected.ID || q.Amount != 127 || q.Currency != "USD" || q.Release != release.ID || q.Version != release.Version || q.Tool != release.Server.Tools[0].Name {
		t.Fatal("price units or immutable binding changed", q)
	}
	auth.registerErr = nil
	if err := m.publishAuthorized(context.Background(), alice, "alice", publish); err != nil {
		t.Fatal(err)
	}
	if auth.registrations[1] != q {
		t.Fatal("publication retry changed quote identity")
	}
	before := len(auth.registrations)
	if err := m.publishAuthorized(context.Background(), bob, "bob", publish); !errors.Is(err, ErrMissing) {
		t.Fatal("other publisher accepted", err)
	}
	if len(auth.registrations) != before {
		t.Fatal("unauthorized publisher reached Core")
	}
}
