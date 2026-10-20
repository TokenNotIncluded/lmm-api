// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package rustbridge

import (
	"context"
	market "github.com/TokenNotIncluded/api.lmm.best/extensions/internal/modules/toolmarket"
)

var _ market.CoreLegacyReports = (*Adapter)(nil)

func (a *Adapter) CreateLegacyReport(ctx context.Context, credential string, in market.LegacyReportRequest) (market.LegacyCallReport, error) {
	var out market.LegacyCallReport
	err := a.requestVersion(ctx, 2, "reports/create", credential, in, &out)
	return out, err
}
func (a *Adapter) ReviewLegacyReport(ctx context.Context, credential string, in market.LegacyReviewReportRequest) (market.LegacyCallReport, error) {
	var out market.LegacyCallReport
	err := a.requestVersion(ctx, 2, "reports/review", credential, in, &out)
	return out, err
}
func (a *Adapter) ListLegacyReports(ctx context.Context, credential string, in market.LegacyReportListRequest) (market.LegacyReportPage, error) {
	var out market.LegacyReportPage
	err := a.requestVersion(ctx, 2, "reports/list", credential, in, &out)
	return out, err
}
func (a *Adapter) ListLegacyIncome(ctx context.Context, credential string, in market.LegacyIncomeRequest) (market.LegacyIncomePage, error) {
	var out market.LegacyIncomePage
	err := a.requestVersion(ctx, 2, "income/list", credential, in, &out)
	return out, err
}
