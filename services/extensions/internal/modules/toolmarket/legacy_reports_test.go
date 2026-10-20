// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package toolmarket

import (
	"context"
	"encoding/json"
	"errors"
	"strings"
	"testing"
)

// Go boundary checks; Core PostgreSQL tests own business acceptance.
type reportAuthority struct {
	*webTestAuthority
	report LegacyCallReport
	income LegacyIncomePage
	fail   error
	calls  int
}

func (a *reportAuthority) CreateLegacyReport(context.Context, string, LegacyReportRequest) (LegacyCallReport, error) {
	a.calls++
	return a.report, a.fail
}
func (a *reportAuthority) ReviewLegacyReport(context.Context, string, LegacyReviewReportRequest) (LegacyCallReport, error) {
	a.calls++
	return a.report, a.fail
}
func (a *reportAuthority) ListLegacyReports(context.Context, string, LegacyReportListRequest) (LegacyReportPage, error) {
	a.calls++
	return LegacyReportPage{[]LegacyCallReport{a.report}}, a.fail
}
func (a *reportAuthority) ListLegacyIncome(context.Context, string, LegacyIncomeRequest) (LegacyIncomePage, error) {
	a.calls++
	return a.income, a.fail
}
func reportFixture(t *testing.T) (*Module, *reportAuthority) {
	t.Helper()
	m, web, _ := webFixture(t)
	a := &reportAuthority{webTestAuthority: web}
	m.authority = a
	now := m.now().Unix()
	a.report = LegacyCallReport{CallID: strings.Repeat("a", 64), UserID: 1, OwnerID: 2, ServiceID: "reports-service", Reason: "wrong delivery", Status: "pending", CreatedAt: now, Evidence: LegacyBillingEvidence{VersionID: "version", ToolID: "tool", BillingPrice: LegacyQuotePrice{Mode: "fixed", PriceQuota: 80}, PricingDigest: strings.Repeat("b", 64), FeeBPS: 2500, RecipientID: 3, Held: 80, Refunded: 80, SettlementStatus: "released", SettlementJournalID: new(int64)}}
	*a.report.Evidence.SettlementJournalID = 7
	return m, a
}
func TestLegacyReportsNeverSubstituteLocalEvidenceOrMissingCore(t *testing.T) {
	m, a := reportFixture(t)
	ctx := context.Background()
	in := LegacyReportRequest{a.report.CallID, " wrong delivery "}
	if got, err := m.LegacyCreateReport(ctx, "1", "1", in); err != nil || got.Reason != "wrong delivery" {
		t.Fatalf("%+v %v", got, err)
	}
	a.report.UserID = 2
	if _, err := m.LegacyCreateReport(ctx, "1", "1", in); !errors.Is(err, ErrDenied) {
		t.Fatal(err)
	}
	a.report.UserID = 1
	a.fail = ErrUnavailable
	if _, err := m.LegacyCreateReport(ctx, "1", "1", in); !errors.Is(err, ErrUnavailable) {
		t.Fatal(err)
	}
	m.authority = &webTestAuthority{}
	if _, err := m.LegacyCreateReport(ctx, "1", "1", in); !errors.Is(err, ErrUnavailable) {
		t.Fatal(err)
	}
}
func TestLegacyReportReviewRejectsPartyAndForgedLifecycle(t *testing.T) {
	m, a := reportFixture(t)
	ctx := context.Background()
	r := &a.report
	r.Status = "confirmed"
	r.ReviewedBy = 3
	r.ReviewNote = "confirmed"
	r.ReviewedAt = r.CreatedAt
	in := LegacyReviewReportRequest{r.CallID, true, "confirmed"}
	if _, err := m.LegacyReviewReport(ctx, "3", "3", in); err != nil {
		t.Fatal(err)
	}
	r.OwnerID = 3
	if _, err := m.LegacyReviewReport(ctx, "3", "3", in); !errors.Is(err, ErrDenied) {
		t.Fatal(err)
	}
	r.OwnerID = 2
	r.ServiceLifecycle = &LegacyServiceLifecycleReceipt{LegacyServiceLifecycleRequest: LegacyServiceLifecycleRequest{OperationID: "review", ServiceID: r.ServiceID, Paused: false}, Suspended: true, Revision: 2, UpdatedBy: 3, UpdatedAt: r.CreatedAt}
	if _, err := m.LegacyReviewReport(ctx, "3", "3", in); !errors.Is(err, ErrDenied) {
		t.Fatal(err)
	}
	r.ServiceLifecycle = nil
	r.UserID = 3
	if _, err := m.LegacyListReports(ctx, "3", "3", 0, 10); !errors.Is(err, ErrDenied) {
		t.Fatal(err)
	}
}
func TestLegacyIncomeUsesCoreHistoricalRecipientAndRate(t *testing.T) {
	m, a := reportFixture(t)
	ctx := context.Background()
	call := strings.Repeat("a", 64)
	a.income = LegacyIncomePage{Owner: Principal{"2", "2"}, Rows: []LegacyIncomeReceipt{{ID: call + ":author", CallID: call, Kind: "author", Quota: 60, FeeBPS: 2500, RecipientID: 2, SellerID: 2, SettlementJournalID: 8, CreatedAt: m.now().Unix()}}}
	if got, err := m.LegacyListIncome(ctx, "2", "2", 0, 10); err != nil || got.Rows[0].FeeBPS != 2500 {
		t.Fatalf("%+v %v", got, err)
	}
	a.income.Rows[0].RecipientID = 1
	if _, err := m.LegacyListIncome(ctx, "2", "2", 0, 10); !errors.Is(err, ErrDenied) {
		t.Fatal(err)
	}
	a.income.Rows[0].RecipientID = 2
	a.income.Rows = append(a.income.Rows, a.income.Rows[0])
	if _, err := m.LegacyListIncome(ctx, "2", "2", 0, 10); !errors.Is(err, ErrDenied) {
		t.Fatal(err)
	}
}
func TestLegacyReportsStrictWireAndBoundedText(t *testing.T) {
	m, a := reportFixture(t)
	raw, err := json.Marshal(a.report)
	if err != nil {
		t.Fatal(err)
	}
	var fields map[string]any
	if err = json.Unmarshal(raw, &fields); err != nil {
		t.Fatal(err)
	}
	evidence := fields["evidence"].(map[string]any)
	delete(evidence, "proof_source")
	missingProof, _ := json.Marshal(fields)
	var missing LegacyCallReport
	if json.Unmarshal(missingProof, &missing) == nil {
		t.Fatal("missing proof provenance accepted")
	}
	evidence["proof_source"] = nil
	delete(evidence, "fee_bps")
	bad, _ := json.Marshal(fields)
	var out LegacyCallReport
	if json.Unmarshal(bad, &out) == nil {
		t.Fatal("missing frozen fee accepted")
	}
	_, err = m.LegacyCreateReport(context.Background(), "1", "1", LegacyReportRequest{a.report.CallID, strings.Repeat("界", 2001)})
	if !errors.Is(err, ErrInvalid) {
		t.Fatal(err)
	}
}

func TestLegacyReportHistoricalACKPreservesNewerLifecycle(t *testing.T) {
	m, a := reportFixture(t)
	r := &a.report
	r.Status = "confirmed"
	r.ReviewedBy = 3
	r.ReviewNote = "confirmed"
	r.ReviewedAt = r.CreatedAt
	r.ServiceLifecycle = &LegacyServiceLifecycleReceipt{LegacyServiceLifecycleRequest: LegacyServiceLifecycleRequest{OperationID: "review", ServiceID: r.ServiceID, Paused: true}, Suspended: true, Revision: 2, UpdatedBy: 3, UpdatedAt: r.CreatedAt}
	if err := m.store.update(func(d *database) error {
		w := webState(d)
		w.Services[r.ServiceID] = WebServiceRecord{ID: r.ServiceID, OwnerID: 2, Status: "published", CoreLifecycleRevision: 5, UpdatedAt: r.CreatedAt + 1}
		return nil
	}); err != nil {
		t.Fatal(err)
	}
	if _, err := m.LegacyReviewReport(context.Background(), "3", "3", LegacyReviewReportRequest{r.CallID, true, "confirmed"}); err != nil {
		t.Fatal(err)
	}
	if err := m.store.view(func(d database) error {
		s := d.Web.Services[r.ServiceID]
		if s.Status != "published" || s.CoreLifecycleRevision != 5 {
			t.Fatalf("historical ACK undid newer lifecycle: %+v", s)
		}
		return nil
	}); err != nil {
		t.Fatal(err)
	}
}

// Every write-capable execution method fails this test if report tries recovery.
type reportReadAuthority struct {
	*reportAuthority
	CoreLegacyCredits
	receipt LegacyCreditCall
	queries int
}

func (a *reportReadAuthority) QueryLegacyCredits(_ context.Context, _ string, id LegacyCreditID) (LegacyCreditCall, error) {
	a.queries++
	if id.Owner != a.receipt.Owner {
		return LegacyCreditCall{}, ErrDenied
	}
	return a.receipt, nil
}
func TestLegacyReportReadsTerminalCoreWithoutExecutionRecovery(t *testing.T) {
	m, base := reportFixture(t)
	a := &reportReadAuthority{reportAuthority: base}
	m.authority = a
	r := LegacyExecution{ID: "local-report", Owner: Principal{"1", "1"}, Input: LegacyExecuteInput{ClientID: "client", RequestKey: "request", ToolID: "tool", VersionID: "version", GrantID: "grant"}, QuoteID: "quote", InputDigest: strings.Repeat("c", 64), State: "running", ResultDigest: strings.Repeat("d", 64)}
	r.Core.ID = base.report.CallID
	a.receipt = LegacyCreditCall{ID: r.Core.ID, Owner: r.Owner, ClientID: r.Input.ClientID, RequestKey: r.Input.RequestKey, QuoteID: r.QuoteID, GrantID: r.Input.GrantID, ToolID: r.Input.ToolID, VersionID: r.Input.VersionID, InputDigest: r.InputDigest, State: "released", Held: 80, Refunded: 80, ResolveBy: m.now().Unix() + 600}
	if err := m.store.update(func(d *database) error {
		if d.LegacyExecutions == nil {
			d.LegacyExecutions = map[string]LegacyExecution{}
		}
		d.LegacyExecutions[r.ID] = r
		return nil
	}); err != nil {
		t.Fatal(err)
	}
	in := WebRequest{Action: "report", ID: r.ID, Body: json.RawMessage(`{"reason":"wrong delivery"}`)}
	if _, err := m.legacyWebReports(context.Background(), "1", "1", in); err != nil {
		t.Fatal(err)
	}
	if a.queries != 1 || a.calls != 1 {
		t.Fatalf("queries %d reports %d", a.queries, a.calls)
	}
	if err := m.store.view(func(d database) error {
		got := d.LegacyExecutions[r.ID]
		if got.State != r.State || got.Core.State != r.Core.State || len(got.RawResult) != 0 {
			t.Fatal("report changed execution")
		}
		return nil
	}); err != nil {
		t.Fatal(err)
	}
	if _, err := m.legacyWebReports(context.Background(), "2", "2", in); !errors.Is(err, ErrMissing) {
		t.Fatal(err)
	}
	if a.queries != 1 || a.calls != 1 {
		t.Fatal("cross-owner read reached Core")
	}
	a.receipt.ID = strings.Repeat("e", 64)
	if _, err := m.legacyWebReports(context.Background(), "1", "1", in); !errors.Is(err, ErrDenied) {
		t.Fatal(err)
	}
	a.receipt.ID = r.Core.ID
	a.receipt.State = "running"
	if _, err := m.legacyWebReports(context.Background(), "1", "1", in); !errors.Is(err, ErrConflict) {
		t.Fatal(err)
	}
	if a.calls != 1 {
		t.Fatal("nonterminal or mismatched receipt reported")
	}
}

func TestLegacyReportProofSourceDoesNotTrustHostUsageLabels(t *testing.T) {
	m, a := reportFixture(t)
	ctx := context.Background()
	in := LegacyReportRequest{a.report.CallID, "wrong delivery"}
	source, usage := "host_witness", "platform_verified"
	a.report.Evidence.ProofSource = &source
	if _, err := m.LegacyCreateReport(ctx, "1", "1", in); err != nil {
		t.Fatal(err)
	}
	a.report.Evidence.UsageSource = &usage
	if _, err := m.LegacyCreateReport(ctx, "1", "1", in); !errors.Is(err, ErrDenied) {
		t.Fatal(err)
	}
	source = "supplier_signed"
	if _, err := m.LegacyCreateReport(ctx, "1", "1", in); !errors.Is(err, ErrDenied) {
		t.Fatal(err)
	}
	usage = "tool_reported"
	if _, err := m.LegacyCreateReport(ctx, "1", "1", in); err != nil {
		t.Fatal(err)
	}
	a.report.Evidence.ProofSource = nil
	if _, err := m.LegacyCreateReport(ctx, "1", "1", in); !errors.Is(err, ErrDenied) {
		t.Fatal(err)
	}
}
