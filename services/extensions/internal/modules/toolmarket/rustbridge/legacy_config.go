// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package rustbridge

import (
	"context"
	market "github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/toolmarket"
)

var _ market.CoreLegacyConfig = (*Adapter)(nil)

func (a *Adapter) ReadLegacyConfig(ctx context.Context) (market.LegacyMarketConfig, error) {
	var out market.LegacyMarketConfig
	err := a.requestVersion(ctx, 2, "config/read", "", struct{}{}, &out)
	return out, err
}
func (a *Adapter) UpdateLegacyConfig(ctx context.Context, credential string, in market.LegacyMarketConfigUpdate) (market.LegacyMarketConfigUpdateReceipt, error) {
	var out market.LegacyMarketConfigUpdateReceipt
	err := a.requestVersion(ctx, 2, "config/update", credential, in, &out)
	return out, err
}
