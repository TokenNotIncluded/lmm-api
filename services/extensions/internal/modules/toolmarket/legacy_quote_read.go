// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package toolmarket

import (
	"context"
	"math"
)

// LegacyReadToolQuote reads the immutable published extension snapshot. This
// never registers a quote or authorizes funds: Core still matches its original
// registered quote and grant when creating/using an assistant scope. Recovery
// must use the retained execution's digests, not this current-publication read.
func (m *Module) LegacyReadToolQuote(ctx context.Context, credential, account, serviceID, versionID, toolID string) (LegacyCreditQuote, error) {
	user, err := m.WebAuthorize(ctx, credential, account, "toolmarket.read")
	if err != nil {
		return LegacyCreditQuote{}, err
	}
	var out LegacyCreditQuote
	err = m.store.view(func(d database) error {
		tool, err := legacyLiveTool(&d, user, toolID, versionID)
		if err != nil {
			return err
		}
		v := d.Web.Versions[versionID]
		if v.Version.ServiceID != serviceID || v.Version.ID != versionID || v.ValidationDigest != v.Digest || v.Version.Digest != v.Digest || !legacyRulesHash(v.RemoteDigests[toolID]) || tool.ProviderPricing != nil {
			return ErrConflict
		}
		if v.Version.ExecutionType == "builtin" {
			if _, err := legacyCompiledExecution(v, tool); err != nil {
				return err
			}
		} else if v.Version.ExecutionType != "remote" {
			return ErrDenied
		}
		price, err := LegacyQuotePriceFromPricing(tool.LegacyPricing())
		if err != nil {
			return err
		}
		priceDigest, err := LegacyQuotePricingDigest(tool.LegacyPricing())
		if err != nil {
			return err
		}
		out = LegacyCreditQuote{ID: digest([]string{"legacy-credit-quote-v2", versionID, toolID, v.RemoteDigests[toolID], priceDigest}), ServiceID: serviceID, VersionID: versionID, ToolID: toolID, RemoteDigest: v.RemoteDigests[toolID], PricingDigest: priceDigest, Price: price, Expires: math.MaxInt64}
		return nil
	})
	return out, err
}
