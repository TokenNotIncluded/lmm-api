// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package toolmarket

import "context"

// QuoteRegistration describes an immutable native USD quote. Conversion and
// publisher admission belong to Core; neither this DTO nor Go grants spending.
type QuoteRegistration struct {
	ID        string `json:"id"`
	Release   string `json:"release"`
	Tool      string `json:"tool"`
	Version   string `json:"version"`
	PriceHash string `json:"price_hash"`
	Amount    int64  `json:"amount"`
	Currency  string `json:"currency"`
	Expires   int64  `json:"expires"`
}

type QuoteRegistrar interface {
	RegisterToolQuote(context.Context, string, QuoteRegistration) error
}

func (m *Module) publishAuthorized(ctx context.Context, p Principal, credential string, in PublishInput) error {
	registrar, remote := m.authority.(QuoteRegistrar)
	if !remote || !in.Published {
		return m.publish(p, in)
	}
	var release Release
	if err := m.store.view(func(d database) error {
		r, ok := d.Releases[in.Release]
		if !ok || r.Server.Owner != p {
			return ErrMissing
		}
		release = r
		return nil
	}); err != nil {
		return err
	}
	// Register before exposing the release. Partial acknowledgements are safe to
	// retry using the same immutable quote IDs; local publication stays disabled.
	for _, tool := range release.Server.Tools {
		quote := quoteFor(release, tool, m.now())
		if !quote.Available {
			return ErrPrice
		}
		if err := registrar.RegisterToolQuote(ctx, credential, QuoteRegistration{ID: quote.ID, Release: release.ID, Tool: tool.Name, Version: release.Version, PriceHash: quote.ID, Amount: quote.Amount, Currency: quote.Currency, Expires: quote.Expires}); err != nil {
			return err
		}
	}
	if err := m.reauthorize(ctx, p, credential, "toolmarket.manage"); err != nil {
		return err
	}
	return m.publish(p, in)
}
