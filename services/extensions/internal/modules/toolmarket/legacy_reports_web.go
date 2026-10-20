// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package toolmarket

import (
	"context"
	"strconv"
)

// The original browser call id is an extension execution id. Resolve it through
// the owner-scoped Core read before using its canonical call id for a report.
func (m *Module) legacyWebReports(ctx context.Context, credential, account string, in WebRequest) (any, error) {
	switch in.Action {
	case "report":
		var body struct {
			Reason string `json:"reason"`
		}
		if err := decodeStrict(in.Body, &body); err != nil {
			return nil, err
		}
		callID, err := m.legacyReportCallID(ctx, credential, account, in.ID)
		if err != nil {
			return nil, err
		}
		return m.LegacyCreateReport(ctx, credential, account, LegacyReportRequest{callID, body.Reason})
	case "reports":
		page, err := m.LegacyListReports(ctx, credential, account, in.Offset, in.Limit)
		return page.Rows, err
	case "report.review":
		var body struct {
			Confirmed bool   `json:"confirmed"`
			Note      string `json:"note"`
		}
		if err := decodeStrict(in.Body, &body); err != nil {
			return nil, err
		}
		if err := legacyRequiredFields(in.Body, "confirmed", "note"); err != nil {
			return nil, err
		}
		return m.LegacyReviewReport(ctx, credential, account, LegacyReviewReportRequest{in.ID, body.Confirmed, body.Note})
	case "income":
		page, err := m.LegacyListIncome(ctx, credential, account, in.Offset, in.Limit)
		return page.Rows, err
	default:
		return nil, ErrMissing
	}
}

// Reporting reads the original receipt without driving execution recovery or
// requiring retained provider output. Only Core owns terminal billing evidence.
func (m *Module) legacyReportCallID(ctx context.Context, credential, account, id string) (string, error) {
	user, err := m.WebAuthorize(ctx, credential, account, "toolmarket.read")
	if err != nil {
		return "", err
	}
	var record LegacyExecution
	err = m.store.view(func(d database) error {
		r, ok := d.LegacyExecutions[id]
		if !ok || r.Owner.User != strconv.FormatInt(user, 10) || r.Owner.Account != account {
			return ErrMissing
		}
		record = r
		return nil
	})
	if err != nil {
		return "", err
	}
	if record.Core.ID == "" {
		return "", ErrConflict
	}
	port, ok := m.authority.(CoreLegacyCredits)
	if !ok {
		return "", ErrUnavailable
	}
	receipt, err := port.QueryLegacyCredits(ctx, credential, LegacyCreditID{Owner: record.Owner, CallID: record.Core.ID})
	if err != nil {
		return "", err
	}
	if receipt.ID != record.Core.ID || !legacyCallMatches(receipt, record) {
		return "", ErrDenied
	}
	if receipt.State != "settled" && receipt.State != "released" {
		return "", ErrConflict
	}
	return receipt.ID, nil
}
