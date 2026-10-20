// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package toolmarket

import (
	"context"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"strings"
	"testing"
	"time"
)

func TestLegacyExecutionProviderBusinessAndPublicProjection(t *testing.T) {
	pricing := LegacyProviderPricing{Provider: "monid", Multiplier: "2"}
	quote := LegacyProviderQuote{Provider: "monid", TargetProvider: "vendor", TargetEndpoint: "operation"}
	pending := json.RawMessage(`{"structuredContent":{"status":"RUNNING","runId":"run_1","provider":"vendor","endpoint":"operation"},"content":[]}`)
	p, v, id := legacyProviderBusinessResult(pricing, quote, pending)
	if !p || v || id != "run_1" {
		t.Fatal(p, v, id)
	}
	bad := json.RawMessage(`{"structuredContent":{"status":"RUNNING","runId":"../history","provider":"vendor","endpoint":"operation"},"content":[]}`)
	p, v, id = legacyProviderBusinessResult(pricing, quote, bad)
	if !p || v || id != "" {
		t.Fatal("unsafe run accepted")
	}
	for _, raw := range []string{
		`{"structuredContent":{"status":"COMPLETED","provider":"vendor","endpoint":"operation","providerResponse":{"httpStatus":500}},"content":[]}`,
		`{"structuredContent":{"status":"COMPLETED","provider":"other","endpoint":"operation","providerResponse":{"httpStatus":200}},"content":[]}`,
	} {
		_, valid, _ := legacyProviderBusinessResult(pricing, quote, json.RawMessage(raw))
		if valid {
			t.Fatal("invalid provider success", raw)
		}
	}
	raw := json.RawMessage(`{"structuredContent":{"status":"COMPLETED","runId":"run_1","provider":"vendor","endpoint":"operation","providerResponse":{"httpStatus":200,"merchant":"hidden-one"},"merchant":"hidden-two","workspace":"hidden-three","output":{"image":"ok"}},"content":[{"type":"text","text":"hidden-four"}]}`)
	clean := legacyMonidPublicResult(raw)
	if strings.Contains(string(clean), "hidden") {
		t.Fatal("merchant metadata leaked", string(clean))
	}
	_, valid, _ := legacyProviderBusinessResult(pricing, quote, raw)
	if !valid {
		t.Fatal("valid final result rejected")
	}
}
func TestLegacyExecutionProviderPollReadsBoundRunOnly(t *testing.T) {
	m, port, r := executionFixture(t)
	pricing := LegacyProviderPricing{Provider: "monid", Multiplier: "2"}
	quote := LegacyProviderQuote{Provider: "monid", TargetProvider: "vendor", TargetEndpoint: "operation"}
	r.ProviderQuote = &quote
	r.ProviderPricing = &pricing
	r.ProviderOwnerID = 1
	r.MeteringBinding = LegacyMeteringBinding{ServiceID: "service", VersionID: "version", ToolID: "tool", Endpoint: "https://mcp.monid.ai/v1"}
	secret := "merchant-private-secret"
	_ = m.store.update(func(d *database) error {
		w := webState(d)
		w.Services["service"] = WebServiceRecord{ID: "service", OwnerID: 1}
		w.Versions["version"] = webSnapshot{Version: WebVersion{ID: "version", ServiceID: "service", ExecutionType: "remote", Endpoint: "https://mcp.monid.ai/v1"}, Tools: []WebTool{{ToolID: "tool", VersionID: "version", Name: "monid_run", ProviderPricing: &pricing}}}
		w.Credentials = map[string]webCredential{"version": {OwnerID: 1, ServiceID: "service", VersionID: "version", EndpointDigest: digest("https://mcp.monid.ai/v1"), Mode: "bearer", Secret: secret}}
		d.LegacyExecutions[r.ID] = r
		return nil
	})
	bound, e := m.legacyBindProviderRun(r, "run_1")
	if e != nil {
		t.Fatal(e)
	}
	if _, e = m.legacyBindProviderRun(bound, "another"); !errors.Is(e, ErrConflict) {
		t.Fatal("provider run rebound", e)
	}
	authority := &executionAuthority{executionCredits: *port}
	m.authority = authority
	gets := 0
	m.net = &network{client: &http.Client{Transport: legacyProviderTransport(func(req *http.Request) (*http.Response, error) {
		gets++
		if req.Method != http.MethodGet || req.URL.String() != "https://api.monid.ai/v1/runs/run_1" || req.Header.Get("Authorization") != "Bearer "+secret {
			t.Fatal("unexpected provider operation", req.Method, req.URL)
		}
		return &http.Response{StatusCode: 200, Header: http.Header{"Content-Type": []string{"application/json"}}, Body: io.NopCloser(strings.NewReader(`{"runId":"run_1","provider":"vendor","endpoint":"operation","status":"COMPLETED","providerResponse":{"httpStatus":200,"merchant":"private"},"workspace":"private","output":"ok"}`))}, nil
	})}}
	m.now = func() time.Time { return time.Unix(103, 0) }
	count, e := m.LegacyRecoverProviderCalls(context.Background(), 10)
	if !errors.Is(e, ErrUnavailable) || count != 0 || gets != 1 || authority.finishes != 0 {
		t.Fatal(count, e, gets, authority.finishes)
	}
	count, e = m.LegacyRecoverProviderCalls(context.Background(), 10)
	if e != nil || count != 0 || gets != 1 {
		t.Fatal("completed provider repeated", count, e, gets)
	}
	_ = m.store.view(func(d database) error {
		got := d.LegacyExecutions[r.ID]
		if got.State != "result" || got.Core.State != "running" || strings.Contains(string(got.Result), "private") {
			t.Fatal(got)
		}
		return nil
	})
}
