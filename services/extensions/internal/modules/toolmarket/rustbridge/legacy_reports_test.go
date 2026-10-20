// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package rustbridge

import (
	"context"
	"encoding/json"
	"fmt"
	market "github.com/TokenNotIncluded/api.lmm.best/extensions/internal/modules/toolmarket"
	"net"
	"net/http"
	"path/filepath"
	"strings"
	"sync/atomic"
	"testing"
)

// A real Unix HTTP transport test, not financial or Core authorization acceptance.
func TestLegacyReportsV2UDSOriginalSessionAndNoRetry(t *testing.T) {
	socket := filepath.Join(t.TempDir(), "r")
	listener, err := net.Listen("unix", socket)
	if err != nil {
		t.Fatal(err)
	}
	host := strings.Repeat("h", 43)
	call := strings.Repeat("a", 64)
	var fail atomic.Bool
	var large atomic.Bool
	var hits atomic.Int64
	evidence := market.LegacyBillingEvidence{VersionID: "version", ToolID: "tool", BillingPrice: market.LegacyQuotePrice{Mode: "fixed"}, PricingDigest: strings.Repeat("b", 64), SettlementStatus: "released"}
	report := market.LegacyCallReport{CallID: call, UserID: 1, OwnerID: 2, ServiceID: "service", Reason: "reason", Evidence: evidence, Status: "pending", CreatedAt: 1}
	server := &http.Server{Handler: http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		hits.Add(1)
		if r.Method != "POST" || r.Header.Get("Authorization") != "Bearer "+host || r.Header.Get("X-Lmm-Protocol") != "2" || r.Header.Get("X-Lmm-User-Credential") != "original-browser-session" {
			t.Error("lost independent host token, protocol or original Session")
			w.WriteHeader(403)
			return
		}
		if fail.Load() {
			w.WriteHeader(503)
			return
		}
		switch r.URL.Path {
		case "/internal/tool-market/v2/reports/create":
			var in market.LegacyReportRequest
			d := json.NewDecoder(r.Body)
			d.DisallowUnknownFields()
			if d.Decode(&in) != nil || in.CallID != call || in.Reason != "reason" {
				t.Error("create DTO changed")
				w.WriteHeader(400)
				return
			}
			_ = json.NewEncoder(w).Encode(report)
		case "/internal/tool-market/v2/reports/review":
			var in market.LegacyReviewReportRequest
			d := json.NewDecoder(r.Body)
			d.DisallowUnknownFields()
			if d.Decode(&in) != nil || in.CallID != call || in.Confirmed || in.Note != "note" {
				t.Error("review DTO changed")
				w.WriteHeader(400)
				return
			}
			_ = json.NewEncoder(w).Encode(report)
		case "/internal/tool-market/v2/reports/list":
			if large.Load() {
				rows := make([]market.LegacyCallReport, 100)
				for i := range rows {
					rows[i] = report
					rows[i].CallID = fmt.Sprintf("%064x", i)
					rows[i].Reason = strings.Repeat("\x00", 2000)
					rows[i].ReviewNote = strings.Repeat("\x00", 2000)
					rows[i].Status = "dismissed"
					rows[i].ReviewedBy = 3
					rows[i].ReviewedAt = 1
				}
				_ = json.NewEncoder(w).Encode(market.LegacyReportPage{Rows: rows})
				return
			}
			_ = json.NewEncoder(w).Encode(market.LegacyReportPage{Rows: []market.LegacyCallReport{report}})
		case "/internal/tool-market/v2/income/list":
			var in market.LegacyIncomeRequest
			d := json.NewDecoder(r.Body)
			d.DisallowUnknownFields()
			if d.Decode(&in) != nil || in.Owner != (market.Principal{User: "2", Account: "2"}) {
				t.Error("income owner changed")
				w.WriteHeader(400)
				return
			}
			_ = json.NewEncoder(w).Encode(market.LegacyIncomePage{Owner: in.Owner, Rows: []market.LegacyIncomeReceipt{}})
		default:
			t.Error("unexpected reports endpoint", r.URL.Path)
			w.WriteHeader(404)
		}
	})}
	done := make(chan error, 1)
	go func() { done <- server.Serve(listener) }()
	t.Cleanup(func() { _ = server.Close(); <-done })
	a, err := New(socket, []byte(host))
	if err != nil {
		t.Fatal(err)
	}
	defer a.Close()
	ctx := context.Background()
	session := "original-browser-session"
	if _, err = a.CreateLegacyReport(ctx, session, market.LegacyReportRequest{CallID: call, Reason: "reason"}); err != nil {
		t.Fatal(err)
	}
	if _, err = a.ReviewLegacyReport(ctx, session, market.LegacyReviewReportRequest{CallID: call, Note: "note"}); err != nil {
		t.Fatal(err)
	}
	if _, err = a.ListLegacyReports(ctx, session, market.LegacyReportListRequest{Limit: 10}); err != nil {
		t.Fatal(err)
	}
	if _, err = a.ListLegacyIncome(ctx, session, market.LegacyIncomeRequest{Owner: market.Principal{User: "2", Account: "2"}, Limit: 10}); err != nil {
		t.Fatal(err)
	}
	if hits.Load() != 4 {
		t.Fatal("unexpected requests", hits.Load())
	}
	// Main permits 100 reports with two 2000-rune text fields. JSON escaping
	// expands valid control characters; this legal page exceeds two MiB.
	large.Store(true)
	if page, err := a.ListLegacyReports(ctx, session, market.LegacyReportListRequest{Limit: 100}); err != nil || len(page.Rows) != 100 {
		t.Fatalf("legal full report page truncated: rows=%d err=%v", len(page.Rows), err)
	}
	fail.Store(true)
	if _, err = a.CreateLegacyReport(ctx, session, market.LegacyReportRequest{CallID: call, Reason: "reason"}); err == nil {
		t.Fatal("lost Core response became success")
	}
	if hits.Load() != 6 {
		t.Fatal("uncertain report was retried", hits.Load())
	}
}
