// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package rustbridge

import (
	"context"
	market "github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/toolmarket"
)

var _ market.CoreLegacyAccess = (*Adapter)(nil)

// Core remains the owner of legacy quota policies, ledger counters and personal
// connection tokens. These methods only carry the closed v2 access DTOs over the
// dedicated UDS with the original credential; they never fall back to v1 USD.
func (a *Adapter) CreateLegacyConsent(ctx context.Context, credential string, in market.CoreLegacyConsentRequest) (market.CoreLegacyConsentReceipt, error) {
	var out market.CoreLegacyConsentReceipt
	err := a.requestVersion(ctx, 2, "access/consent-create", credential, in, &out)
	return out, err
}
func (a *Adapter) RevokeLegacyConsent(ctx context.Context, credential string, in market.CoreLegacyIDRequest) (market.CoreLegacyRevocationReceipt, error) {
	var out market.CoreLegacyRevocationReceipt
	err := a.requestVersion(ctx, 2, "access/consent-revoke", credential, in, &out)
	return out, err
}
func (a *Adapter) SetLegacyBudget(ctx context.Context, credential string, in market.CoreLegacyBudgetRequest) (market.CoreLegacyBudgetReceipt, error) {
	var out market.CoreLegacyBudgetReceipt
	err := a.requestVersion(ctx, 2, "access/budget-set", credential, in, &out)
	return out, err
}
func (a *Adapter) CreateLegacyToken(ctx context.Context, credential string, in market.CoreLegacyTokenRequest) (market.CoreLegacyTokenReceipt, error) {
	var out market.CoreLegacyTokenReceipt
	err := a.requestVersion(ctx, 2, "access/token-create", credential, in, &out)
	return out, err
}
func (a *Adapter) RevokeLegacyToken(ctx context.Context, credential string, in market.CoreLegacyIDRequest) (market.CoreLegacyRevocationReceipt, error) {
	var out market.CoreLegacyRevocationReceipt
	err := a.requestVersion(ctx, 2, "access/token-revoke", credential, in, &out)
	return out, err
}
func (a *Adapter) ValidateLegacyToken(ctx context.Context, credential string, in market.CoreLegacyValidateRequest) (market.CoreLegacyTokenReceipt, error) {
	var out market.CoreLegacyTokenReceipt
	err := a.requestVersion(ctx, 2, "access/token-validate", credential, in, &out)
	return out, err
}
func (a *Adapter) DisconnectLegacyClient(ctx context.Context, credential string, in market.CoreLegacyClientRequest) (market.CoreLegacyDisconnectReceipt, error) {
	var out market.CoreLegacyDisconnectReceipt
	err := a.requestVersion(ctx, 2, "access/client-disconnect", credential, in, &out)
	return out, err
}

func (a *Adapter) ListLegacyConsents(ctx context.Context, credential string, in market.CoreLegacyListRequest) (market.CoreLegacyConsentList, error) {
	var out market.CoreLegacyConsentList
	err := a.requestVersion(ctx, 2, "access/consent-list", credential, in, &out)
	return out, err
}
func (a *Adapter) ListLegacyBudgets(ctx context.Context, credential string, in market.CoreLegacyListRequest) (market.CoreLegacyBudgetList, error) {
	var out market.CoreLegacyBudgetList
	err := a.requestVersion(ctx, 2, "access/budget-list", credential, in, &out)
	return out, err
}
func (a *Adapter) ListLegacyTokens(ctx context.Context, credential string, in market.CoreLegacyListRequest) (market.CoreLegacyTokenList, error) {
	var out market.CoreLegacyTokenList
	err := a.requestVersion(ctx, 2, "access/token-list", credential, in, &out)
	return out, err
}
