// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package rustbridge

import (
	"context"
	market "github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/toolmarket"
)

var _ market.CoreLegacyMarketMeta = (*Adapter)(nil)

func (a *Adapter) ReadLegacyMarketMeta(ctx context.Context, raw string) (market.LegacyMarketMetaView, error) {
	var out market.LegacyMarketMetaView
	err := a.requestVersion(ctx, 2, "meta/read", raw, struct{}{}, &out)
	return out, err
}
func (a *Adapter) ReadLegacyMarketMetaOwner(ctx context.Context, raw string, in market.LegacyMarketMetaTarget) (market.LegacyMarketMetaOwnerView, error) {
	var out market.LegacyMarketMetaOwnerView
	err := a.requestVersion(ctx, 2, "meta/owner-read", raw, in, &out)
	return out, err
}
func (a *Adapter) SetLegacyMarketMetaOwner(ctx context.Context, raw string, in market.LegacyMarketMetaSet) (market.LegacyMarketMetaOwnerView, error) {
	var out market.LegacyMarketMetaOwnerView
	err := a.requestVersion(ctx, 2, "meta/owner-set", raw, in, &out)
	return out, err
}
func (a *Adapter) AuthorizeLegacyMarketMeta(ctx context.Context, raw string, in market.LegacyMarketMetaAuthorize) (market.CoreLegacyConsentReceipt, error) {
	var out market.CoreLegacyConsentReceipt
	err := a.requestVersion(ctx, 2, "meta/authorize", raw, in, &out)
	return out, err
}
func (a *Adapter) TightenLegacyMarketClientBudget(ctx context.Context, raw string, in market.LegacyMarketMetaClientBudget) (market.CoreLegacyBudgetReceipt, error) {
	var out market.CoreLegacyBudgetReceipt
	err := a.requestVersion(ctx, 2, "meta/client-budget", raw, in, &out)
	return out, err
}
func (a *Adapter) TightenLegacyMarketToolBudget(ctx context.Context, raw string, in market.LegacyMarketMetaToolBudget) (market.CoreLegacyConsentReceipt, error) {
	var out market.CoreLegacyConsentReceipt
	err := a.requestVersion(ctx, 2, "meta/tool-budget", raw, in, &out)
	return out, err
}
func (a *Adapter) ReadLegacyMarketUsage(ctx context.Context, raw string, in market.LegacyMarketMetaPage) (market.LegacyMarketMetaUsage, error) {
	var out market.LegacyMarketMetaUsage
	err := a.requestVersion(ctx, 2, "meta/usage", raw, in, &out)
	return out, err
}
func (a *Adapter) ReadLegacyMarketCalls(ctx context.Context, raw string, in market.LegacyMarketMetaPage) (market.LegacyMarketMetaCalls, error) {
	var out market.LegacyMarketMetaCalls
	err := a.requestVersion(ctx, 2, "meta/calls", raw, in, &out)
	return out, err
}

var _ market.CoreLegacyMarketOAuthClients = (*Adapter)(nil)

func (a *Adapter) ListLegacyMarketOAuthClients(ctx context.Context, raw string, in market.LegacyMarketOAuthClientsRequest) (market.LegacyMarketOAuthClients, error) {
	var out market.LegacyMarketOAuthClients
	err := a.requestVersion(ctx, 2, "meta/oauth-clients", raw, in, &out)
	return out, err
}
