// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
// Original main 3948a0a apps/api-go/model/tool_market_{reports,queries}.go.
package toolmarket

import (
	"bytes"
	"context"
	"encoding/json"
	"io"
	"strings"
	"unicode/utf8"
)

type LegacyReportRequest struct {
	CallID string `json:"call_id"`
	Reason string `json:"reason"`
}
type LegacyReviewReportRequest struct {
	CallID    string `json:"call_id"`
	Confirmed bool   `json:"confirmed"`
	Note      string `json:"note"`
}
type LegacyReportListRequest struct {
	Offset int `json:"offset"`
	Limit  int `json:"limit"`
}
type LegacyIncomeRequest struct {
	Owner  Principal `json:"owner"`
	Offset int       `json:"offset"`
	Limit  int       `json:"limit"`
}
type LegacyBillingEvidence struct {
	VersionID           string           `json:"version_id"`
	ToolID              string           `json:"tool_id"`
	BillingPrice        LegacyQuotePrice `json:"billing_price"`
	PricingDigest       string           `json:"pricing_digest"`
	FeeBPS              int64            `json:"fee_bps"`
	RecipientID         int64            `json:"recipient_id"`
	Held                CreditAmount     `json:"held_quota"`
	Charged             CreditAmount     `json:"charged_quota"`
	Fee                 CreditAmount     `json:"fee_quota"`
	Seller              CreditAmount     `json:"seller_quota"`
	Refunded            CreditAmount     `json:"refunded_quota"`
	ProofSource         *string          `json:"proof_source"`
	UsageSource         *string          `json:"usage_source"`
	UsageQuantities     map[string]int64 `json:"usage_quantities"`
	ResultDigest        string           `json:"result_digest"`
	SettlementStatus    string           `json:"settlement_status"`
	SettlementJournalID *int64           `json:"settlement_journal_id"`
}
type LegacyCallReport struct {
	CallID           string                         `json:"call_id"`
	UserID           int64                          `json:"user_id"`
	ServiceID        string                         `json:"service_id"`
	OwnerID          int64                          `json:"owner_id"`
	Reason           string                         `json:"reason"`
	Evidence         LegacyBillingEvidence          `json:"evidence"`
	Status           string                         `json:"status"`
	ReviewNote       string                         `json:"review_note"`
	ReviewedBy       int64                          `json:"reviewed_by"`
	CreatedAt        int64                          `json:"created_at"`
	ReviewedAt       int64                          `json:"reviewed_at"`
	ServiceLifecycle *LegacyServiceLifecycleReceipt `json:"service_lifecycle"`
}
type LegacyReportPage struct {
	Rows []LegacyCallReport `json:"rows"`
}
type LegacyIncomeReceipt struct {
	ID                  string       `json:"id"`
	CallID              string       `json:"call_id"`
	Kind                string       `json:"kind"`
	Quota               CreditAmount `json:"quota"`
	FeeBPS              int64        `json:"fee_bps"`
	RecipientID         int64        `json:"recipient_id"`
	SellerID            int64        `json:"seller_id"`
	SettlementJournalID int64        `json:"settlement_journal_id"`
	CreatedAt           int64        `json:"created_at"`
}
type LegacyIncomePage struct {
	Owner Principal             `json:"owner"`
	Rows  []LegacyIncomeReceipt `json:"rows"`
}
type CoreLegacyReports interface {
	CreateLegacyReport(context.Context, string, LegacyReportRequest) (LegacyCallReport, error)
	ReviewLegacyReport(context.Context, string, LegacyReviewReportRequest) (LegacyCallReport, error)
	ListLegacyReports(context.Context, string, LegacyReportListRequest) (LegacyReportPage, error)
	ListLegacyIncome(context.Context, string, LegacyIncomeRequest) (LegacyIncomePage, error)
}

func legacyReportDigest(s string) bool {
	if len(s) != 64 {
		return false
	}
	for i := 0; i < len(s); i++ {
		b := s[i]
		if !(b >= '0' && b <= '9' || b >= 'a' && b <= 'f') {
			return false
		}
	}
	return true
}
func legacyReportText(s string) (string, error) {
	s = strings.TrimSpace(s)
	if s == "" || !utf8.ValidString(s) || utf8.RuneCountInString(s) > 2000 {
		return "", ErrInvalid
	}
	return s, nil
}
func validLegacyReport(r LegacyCallReport, now int64) bool {
	e := r.Evidence
	if !legacyReportDigest(r.CallID) || r.UserID <= 0 || r.OwnerID < 0 || !validID.MatchString(r.ServiceID) || r.CreatedAt <= 0 || r.CreatedAt > now {
		return false
	}
	if t, err := legacyReportText(r.Reason); err != nil || t != r.Reason {
		return false
	}
	if !validID.MatchString(e.VersionID) || !validID.MatchString(e.ToolID) || !legacyReportDigest(e.PricingDigest) || !e.Held.Valid() || !e.Charged.Valid() || !e.Fee.Valid() || !e.Seller.Valid() || !e.Refunded.Valid() || e.Charged > e.Held || e.Refunded != e.Held-e.Charged || e.FeeBPS < 0 || e.FeeBPS > 10000 || e.RecipientID < 0 || e.Fee > 0 && e.RecipientID == 0 {
		return false
	}
	ceiling, err := e.BillingPrice.Ceiling()
	if err != nil || ceiling != e.Held {
		return false
	}
	// Proof provenance comes from Core's immutable proof tables. A host witness
	// proves delivery; it does not turn host-declared usage into platform usage.
	if e.ProofSource == nil {
		if e.UsageSource != nil {
			return false
		}
	} else {
		switch *e.ProofSource {
		case "supplier_signed":
			if e.UsageSource == nil || *e.UsageSource != "tool_reported" {
				return false
			}
		case "host_witness":
			if e.UsageSource != nil {
				return false
			}
		default:
			return false
		}
	}
	if e.SettlementStatus == "released" {
		if e.Charged != 0 || e.Fee != 0 || e.Seller != 0 {
			return false
		}
	} else if e.SettlementStatus == "settled" {
		seller, fee, err := LegacyCreditSplit(e.Charged, e.FeeBPS)
		if err != nil || seller != e.Seller || fee != e.Fee || !legacyReportDigest(e.ResultDigest) {
			return false
		}
	} else {
		return false
	}
	if e.Held > 0 && (e.SettlementJournalID == nil || *e.SettlementJournalID <= 0) {
		return false
	}
	if r.Status == "pending" {
		return r.ReviewedBy == 0 && r.ReviewedAt == 0 && r.ReviewNote == "" && r.ServiceLifecycle == nil
	}
	if r.Status != "confirmed" && r.Status != "dismissed" {
		return false
	}
	if t, err := legacyReportText(r.ReviewNote); err != nil || t != r.ReviewNote {
		return false
	}
	if r.ReviewedBy <= 0 || r.ReviewedBy == r.UserID || r.ReviewedBy == r.OwnerID || r.ReviewedAt < r.CreatedAt || r.ReviewedAt > now {
		return false
	}
	if r.ServiceLifecycle != nil {
		a := r.ServiceLifecycle
		if r.Status != "confirmed" || a.ServiceID != r.ServiceID || !a.Paused || !a.Suspended || a.Deleted || a.UpdatedBy != r.ReviewedBy || a.UpdatedAt != r.ReviewedAt || a.Revision < 2 {
			return false
		}
	}
	return true
}
func (r *LegacyReportPage) UnmarshalJSON(raw []byte) error {
	// Responses may contain 100 reports, each with two 2000-rune text fields.
	// Keep request limits unchanged and apply the separate bounded response limit.
	if len(raw) == 0 || len(raw) > 4<<20 || !utf8.Valid(raw) {
		return ErrInvalid
	}
	if err := uniqueJSON(raw); err != nil {
		return err
	}
	type alias LegacyReportPage
	var v alias
	d := json.NewDecoder(bytes.NewReader(raw))
	d.UseNumber()
	d.DisallowUnknownFields()
	if err := d.Decode(&v); err != nil {
		return ErrInvalid
	}
	var extra any
	if d.Decode(&extra) != io.EOF || len(v.Rows) > 100 {
		return ErrInvalid
	}
	if err := legacyRequiredFields(raw, "rows"); err != nil {
		return err
	}
	*r = LegacyReportPage(v)
	return nil
}
func (r *LegacyIncomePage) UnmarshalJSON(raw []byte) error {
	type alias LegacyIncomePage
	var v alias
	if err := decodeStrict(raw, &v); err != nil {
		return err
	}
	if err := legacyRequiredFields(raw, "owner", "rows"); err != nil {
		return err
	}
	*r = LegacyIncomePage(v)
	return nil
}
func (e *LegacyBillingEvidence) UnmarshalJSON(raw []byte) error {
	type alias LegacyBillingEvidence
	var v alias
	if err := decodeStrict(raw, &v); err != nil {
		return err
	}
	if err := legacyRequiredFields(raw, "version_id", "tool_id", "billing_price", "pricing_digest", "fee_bps", "recipient_id", "held_quota", "charged_quota", "fee_quota", "seller_quota", "refunded_quota", "result_digest", "settlement_status"); err != nil {
		return err
	}
	var fields map[string]json.RawMessage
	if err := json.Unmarshal(raw, &fields); err != nil {
		return ErrInvalid
	}
	// Explicit null means no accepted proof; absence is an older/incomplete DTO.
	if _, ok := fields["proof_source"]; !ok {
		return ErrInvalid
	}
	*e = LegacyBillingEvidence(v)
	return nil
}
func (r *LegacyCallReport) UnmarshalJSON(raw []byte) error {
	type alias LegacyCallReport
	var v alias
	if err := decodeStrict(raw, &v); err != nil {
		return err
	}
	if err := legacyRequiredFields(raw, "call_id", "user_id", "service_id", "owner_id", "reason", "evidence", "status", "review_note", "reviewed_by", "created_at", "reviewed_at"); err != nil {
		return err
	}
	*r = LegacyCallReport(v)
	return nil
}
func (r *LegacyIncomeReceipt) UnmarshalJSON(raw []byte) error {
	type alias LegacyIncomeReceipt
	var v alias
	if err := decodeStrict(raw, &v); err != nil {
		return err
	}
	if err := legacyRequiredFields(raw, "id", "call_id", "kind", "quota", "fee_bps", "recipient_id", "seller_id", "settlement_journal_id", "created_at"); err != nil {
		return err
	}
	*r = LegacyIncomeReceipt(v)
	return nil
}
func (m *Module) reportsPort() (CoreLegacyReports, error) {
	if m == nil || m.authority == nil {
		return nil, ErrUnavailable
	}
	p, ok := m.authority.(CoreLegacyReports)
	if !ok || missingInterface(p) {
		return nil, ErrUnavailable
	}
	return p, nil
}
func (m *Module) LegacyCreateReport(ctx context.Context, credential, account string, in LegacyReportRequest) (LegacyCallReport, error) {
	if !legacyReportDigest(in.CallID) {
		return LegacyCallReport{}, ErrInvalid
	}
	reason, err := legacyReportText(in.Reason)
	if err != nil {
		return LegacyCallReport{}, err
	}
	in.Reason = reason
	user, err := m.WebAuthorize(ctx, credential, account, "toolmarket.read")
	if err != nil {
		return LegacyCallReport{}, err
	}
	port, err := m.reportsPort()
	if err != nil {
		return LegacyCallReport{}, err
	}
	out, err := port.CreateLegacyReport(ctx, credential, in)
	if err != nil {
		return LegacyCallReport{}, err
	}
	if out.CallID != in.CallID || out.UserID != user || out.Reason != in.Reason || !validLegacyReport(out, m.now().Unix()) {
		return LegacyCallReport{}, ErrDenied
	}
	if _, err = m.WebAuthorize(ctx, credential, account, "toolmarket.read"); err != nil {
		return LegacyCallReport{}, err
	}
	return out, nil
}
func (m *Module) LegacyReviewReport(ctx context.Context, credential, account string, in LegacyReviewReportRequest) (LegacyCallReport, error) {
	if !legacyReportDigest(in.CallID) {
		return LegacyCallReport{}, ErrInvalid
	}
	note, err := legacyReportText(in.Note)
	if err != nil {
		return LegacyCallReport{}, err
	}
	in.Note = note
	user, err := m.WebAuthorize(ctx, credential, account, "toolmarket.review")
	if err != nil {
		return LegacyCallReport{}, err
	}
	port, err := m.reportsPort()
	if err != nil {
		return LegacyCallReport{}, err
	}
	out, err := port.ReviewLegacyReport(ctx, credential, in)
	if err != nil {
		return LegacyCallReport{}, err
	}
	status := "dismissed"
	if in.Confirmed {
		status = "confirmed"
	}
	if out.CallID != in.CallID || out.Status != status || out.ReviewedBy != user || out.ReviewNote != in.Note || !validLegacyReport(out, m.now().Unix()) {
		return LegacyCallReport{}, ErrDenied
	}
	if _, err = m.WebAuthorize(ctx, credential, account, "toolmarket.review"); err != nil {
		return LegacyCallReport{}, err
	}
	// Apply only a Core acknowledgement; a delayed review cannot undo a newer lifecycle.
	if a := out.ServiceLifecycle; a != nil {
		err = m.store.update(func(d *database) error {
			w := webState(d)
			s, ok := w.Services[a.ServiceID]
			if !ok {
				return nil
			}
			if s.OwnerID != out.OwnerID {
				return ErrDenied
			}
			// A recovered review receipt remains historical truth after a later
			// administrator lifecycle action; preserve that newer projection.
			if a.Revision < s.CoreLifecycleRevision {
				return nil
			}
			if err := legacyValidateLifecycleAck(s.CoreLifecycleRevision, s.Status, *a); err != nil {
				return err
			}
			s.Status = legacyLifecycleStatus(*a)
			s.CoreLifecycleRevision = a.Revision
			s.UpdatedAt = a.UpdatedAt
			w.Services[s.ID] = s
			return nil
		})
		if err != nil {
			return LegacyCallReport{}, err
		}
	}
	return out, nil
}
func (m *Module) LegacyListReports(ctx context.Context, credential, account string, offset, limit int) (LegacyReportPage, error) {
	if !validWebPage(offset, limit) {
		return LegacyReportPage{}, ErrInvalid
	}
	user, err := m.WebAuthorize(ctx, credential, account, "toolmarket.review")
	if err != nil {
		return LegacyReportPage{}, err
	}
	port, err := m.reportsPort()
	if err != nil {
		return LegacyReportPage{}, err
	}
	out, err := port.ListLegacyReports(ctx, credential, LegacyReportListRequest{offset, limit})
	if err != nil {
		return LegacyReportPage{}, err
	}
	if len(out.Rows) > limit {
		return LegacyReportPage{}, ErrDenied
	}
	seen := map[string]bool{}
	for _, r := range out.Rows {
		if r.UserID == user || r.OwnerID == user || seen[r.CallID] || !validLegacyReport(r, m.now().Unix()) {
			return LegacyReportPage{}, ErrDenied
		}
		seen[r.CallID] = true
	}
	if _, err = m.WebAuthorize(ctx, credential, account, "toolmarket.review"); err != nil {
		return LegacyReportPage{}, err
	}
	return out, nil
}
func (m *Module) LegacyListIncome(ctx context.Context, credential, account string, offset, limit int) (LegacyIncomePage, error) {
	if !validWebPage(offset, limit) {
		return LegacyIncomePage{}, ErrInvalid
	}
	user, err := m.WebAuthorize(ctx, credential, account, "toolmarket.read")
	if err != nil {
		return LegacyIncomePage{}, err
	}
	port, err := m.reportsPort()
	if err != nil {
		return LegacyIncomePage{}, err
	}
	owner := Principal{User: account, Account: account}
	out, err := port.ListLegacyIncome(ctx, credential, LegacyIncomeRequest{owner, offset, limit})
	if err != nil {
		return LegacyIncomePage{}, err
	}
	if out.Owner != owner || len(out.Rows) > limit {
		return LegacyIncomePage{}, ErrDenied
	}
	seen := map[string]bool{}
	for _, r := range out.Rows {
		if !legacyReportDigest(r.CallID) || r.ID != r.CallID+":"+r.Kind || seen[r.ID] || (r.Kind != "author" && r.Kind != "platform") || r.RecipientID != user || r.SellerID <= 0 || !r.Quota.Valid() || r.Quota <= 0 || r.FeeBPS < 0 || r.FeeBPS > 10000 || r.SettlementJournalID <= 0 || r.CreatedAt <= 0 || r.CreatedAt > m.now().Unix() {
			return LegacyIncomePage{}, ErrDenied
		}
		seen[r.ID] = true
	}
	if _, err = m.WebAuthorize(ctx, credential, account, "toolmarket.read"); err != nil {
		return LegacyIncomePage{}, err
	}
	return out, nil
}
