// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package rustbridge

import (
	"context"
	"errors"
	market "github.com/TokenNotIncluded/lmm-api/services/extensions/internal/modules/toolmarket"
)

var _ market.CoreLegacyMarketOAuth = (*Adapter)(nil)

func (a *Adapter) AuthorizeLegacyMarketOAuth(ctx context.Context, in market.LegacyMarketOAuthRequest) (market.LegacyMarketPrincipal, error) {
	var out market.LegacyMarketPrincipal
	err := a.requestVersion(ctx, 1, "oauth-authorize", "", in, &out)
	if errors.Is(err, market.ErrDenied) {
		return market.LegacyMarketPrincipal{}, market.ErrLegacyMarketScope
	}
	return out, err
}
