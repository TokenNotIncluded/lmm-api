// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package rustbridge

import (
	"context"
	market "github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/toolmarket"
)

var _ market.CoreLegacyLifecycle = (*Adapter)(nil)

func (a *Adapter) SetLegacyServiceLifecycle(ctx context.Context, credential string, in market.LegacyServiceLifecycleRequest) (market.LegacyServiceLifecycleReceipt, error) {
	var out market.LegacyServiceLifecycleReceipt
	err := a.requestVersion(ctx, 2, "services/lifecycle", credential, in, &out)
	return out, err
}
