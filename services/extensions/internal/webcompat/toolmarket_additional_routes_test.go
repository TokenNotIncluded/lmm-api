package webcompat

import (
	"net/http/httptest"
	"strings"
	"testing"
)

func TestMarketReportIncomeAndDelegationRoutesPreserveAuthority(t *testing.T) {
	for _, tc := range []struct{ method, path, body, action, permission, id, kind string }{
		{"POST", "/api/tool-market/connection/check", `{"provider":"treg","endpoint":"https://treg.to/mcp/","authentication":{"mode":"bearer","secret":"fixture-team-key"}}`, "connection.check", "toolmarket.manage", "", ""},
		{"GET", "/api/tool-market/executions/by-request/original-request", "", "call.lookup", "toolmarket.read", "original-request", ""},
		{"POST", "/api/tool-market/calls/call-1/resume", `{"request_state":"` + strings.Repeat("a", 64) + `"}`, "call.resume", "toolmarket.execute", "call-1", ""},
		{"POST", "/api/tool-market/calls/call-1/report", `{"reason":"reason"}`, "report", "toolmarket.read", "call-1", ""},
		{"GET", "/api/tool-market/reports?limit=100", "", "reports", "toolmarket.review", "", ""},
		{"POST", "/api/tool-market/reports/call-1/review", `{"confirmed":true,"note":"note"}`, "report.review", "toolmarket.review", "call-1", ""},
		{"GET", "/api/tool-market/income", "", "income", "toolmarket.read", "", ""},
		{"GET", "/api/tool-market/meta-delegations/client/oauth:client-1", "", "delegation.get", "toolmarket.read", "oauth:client-1", "client"},
		{"PUT", "/api/tool-market/meta-delegations/client/oauth:client-1", `{"enabled":true,"max_total_quota":100}`, "delegation.set", "toolmarket.authorize", "oauth:client-1", "client"},
		{"GET", "/api/tool-market/meta-delegations/oauth-clients", "", "delegation.clients", "toolmarket.read", "", ""},
	} {
		t.Run(tc.action, func(t *testing.T) {
			core, service := &authCore{}, &marketTransportFake{}
			r := marketHTTP(tc.method, tc.path, tc.body, true)
			r.Header.Set("X-LMM-User-ID", "999")
			r.Header.Set("X-LMM-Account-ID", "999")
			r.Header.Set("Idempotency-Key", "original-operation")
			rec := httptest.NewRecorder()
			marketMux(t, core, service).ServeHTTP(rec, r)
			if rec.Code != 200 || core.calls != 1 || service.authCalls != 1 || service.businessCalls != 1 || service.token != webToken || service.account != "17" || service.permission != tc.permission || service.request.Action != tc.action || service.request.ID != tc.id || service.request.Kind != tc.kind || service.request.OperationID != "original-operation" || string(service.request.Body) != tc.body {
				t.Fatal(rec.Code, rec.Body.String(), service)
			}
		})
	}
}

func TestMarketDelegationBodyHasIndependentOneKiBBoundary(t *testing.T) {
	core, service := &authCore{}, &marketTransportFake{}
	body := `{"enabled":true,"max_total_quota":1,"padding":"` + strings.Repeat("x", 1024) + `"}`
	r := marketHTTP("PUT", "/api/tool-market/meta-delegations/client/client-1", body, true)
	rec := httptest.NewRecorder()
	marketMux(t, core, service).ServeHTTP(rec, r)
	if rec.Code != 422 || service.authCalls != 1 || service.businessCalls != 0 {
		t.Fatal("oversized delegation reached domain", rec.Code, service)
	}
}
