// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package rustbridge

import (
	"context"
	market "github.com/TokenNotIncluded/lmm-api/services/extensions/internal/modules/toolmarket"
)

var _ market.CoreLegacyQuotes = (*Adapter)(nil)

func (a *Adapter) RegisterLegacyQuote(ctx context.Context, credential string, in market.LegacyCreditQuote) (market.LegacyCreditQuote, error) {
	var out market.LegacyCreditQuote
	err := a.requestVersion(ctx, 2, "quotes", credential, in, &out)
	return out, err
}
func (a *Adapter) ApproveLegacyQuote(ctx context.Context, credential string, in market.LegacyQuoteApproval) (market.LegacyQuoteApprovalReceipt, error) {
	var out market.LegacyQuoteApprovalReceipt
	err := a.requestVersion(ctx, 2, "quotes/approve", credential, in, &out)
	return out, err
}
