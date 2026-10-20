// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package toolmarket

import (
	"bytes"
	"context"
	"encoding/base64"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"strings"
	"testing"
	"time"
)

const meteredFixtureSession = "fixture-metered-private-session-token"

type meteredExecutionAuthority struct{ executionAuthority }

func (a *meteredExecutionAuthority) Authorize(ctx context.Context, credential, account, action string) (Principal, error) {
	if credential != meteredFixtureSession {
		return Principal{}, ErrAuth
	}
	return a.webTestAuthority.Authorize(ctx, "1", account, action)
}

// This fixture proves consumer ordering and immutable forwarding. The fake Core
// makes no cryptographic or financial acceptance claim; real Core/PG verifies it.
func meteredExecutionFixture(t *testing.T) (*Module, *meteredExecutionAuthority, LegacyExecuteInput, *int, *int) {
	t.Helper()
	remote := Tool{Name: "echo", InputSchema: json.RawMessage(`{"type":"object"}`)}
	tool := WebTool{ToolID: "tool", VersionID: "version", Name: "echo", InputSchema: `{"type":"object"}`, BillingMode: "metered", BillingRules: []LegacyBillingRule{{Metric: "images", RateQuota: 2, MaxQuantity: 5}}, PriceQuota: 10}
	d := emptyDB()
	w := webState(&d)
	w.Services["service"] = WebServiceRecord{ID: "service", OwnerID: 1, LiveVersionID: "version", Status: "published"}
	w.Versions["version"] = webSnapshot{Version: WebVersion{ID: "version", ServiceID: "service", Status: "published", ExecutionType: "remote", Visibility: "public", Endpoint: "https://tools.example.test/mcp"}, Digest: "snapshot", RemoteDigests: map[string]string{"tool": digest(remote)}, Tools: []WebTool{tool}}
	w.Installations[digest([]any{int64(1), "web", "tool"})] = WebInstallation{UserID: 1, ClientID: "web", ToolID: "tool", VersionID: "version"}
	legacyAccess(&d).Grants["grant"] = LegacyConsentGrant{ID: "grant", UserID: 1, ClientID: "web", ToolID: "tool", VersionID: "version", QuoteID: "quote", ExpiresAt: 1000}
	input := LegacyExecuteInput{ClientID: "web", RequestKey: "once", ToolID: "tool", VersionID: "version", GrantID: "grant", Arguments: json.RawMessage(`{}`)}
	pricing, _ := LegacyQuotePricingDigest(tool.LegacyPricing())
	authority := &meteredExecutionAuthority{executionAuthority: executionAuthority{executionCredits: executionCredits{r: LegacyCreditCall{ID: digest([]any{int64(1), "web", "once"}), Owner: Principal{User: "1", Account: "1"}, ClientID: "web", RequestKey: "once", QuoteID: "quote", GrantID: "grant", ToolID: "tool", VersionID: "version", InputDigest: digest(map[string]any{}), PricingDigest: pricing, ResolveBy: 220, Held: 10, State: "running", DispatchID: "dispatch", DispatchClaimed: true}}}}
	m, err := New(&Store{data: d}, authority, Config{WebOnly: true})
	if err != nil {
		t.Fatal(err)
	}
	m.now = func() time.Time { return time.Unix(100, 0) }
	network, calls := new(int), new(int)
	m.net.client = &http.Client{Transport: legacyProviderTransport(func(r *http.Request) (*http.Response, error) {
		*network++
		if authority.reserves != 1 {
			t.Error("supplier network before Core reserve admission")
		}
		var req struct {
			ID     json.RawMessage `json:"id"`
			Method string          `json:"method"`
			Params struct {
				Meta struct {
					Metering LegacyMeteringContext `json:"lmm_metering"`
				} `json:"_meta"`
			} `json:"params"`
		}
		if json.NewDecoder(r.Body).Decode(&req) != nil {
			t.Fatal("invalid RPC")
		}
		var result any
		switch req.Method {
		case "initialize":
			result = map[string]any{"protocolVersion": ProtocolVersion, "capabilities": map[string]any{"tools": map[string]any{}}}
		case "notifications/initialized":
			return &http.Response{StatusCode: 202, Header: http.Header{}, Body: io.NopCloser(strings.NewReader(""))}, nil
		case "tools/list":
			result = map[string]any{"tools": []Tool{remote}}
		case "tools/call":
			*calls++
			ctx := req.Params.Meta.Metering
			if ctx.CallID != authority.r.ID || ctx.InputDigest != authority.r.InputDigest || ctx.PricingDigest == pricing {
				t.Error("supplier context mixed Core price hash")
			}
			result = map[string]any{"content": []any{map[string]any{"type": "text", "text": "ok"}}, "_meta": map[string]any{"lmm_metering": map[string]any{"adapter_id": "core-configured-only", "context": ctx, "signature": "supplier-opaque-signature", "quantities": map[string]int64{"images": 3}, "result_digest": "supplier-result-digest"}}}
		default:
			t.Fatal(req.Method)
		}
		raw, _ := json.Marshal(map[string]any{"jsonrpc": "2.0", "id": req.ID, "result": result})
		return &http.Response{StatusCode: 200, Header: http.Header{"Content-Type": []string{"application/json"}}, Body: io.NopCloser(strings.NewReader(string(raw)))}, nil
	})}
	return m, authority, input, network, calls
}
func TestLegacyExecutionMeteredReserveRejectsBeforeAnySupplierNetwork(t *testing.T) {
	m, port, input, network, calls := meteredExecutionFixture(t)
	port.reserveErr = ErrDenied
	for i := 0; i < 2; i++ {
		if _, err := m.LegacyExecuteRemote(context.Background(), meteredFixtureSession, "1", input); !errors.Is(err, ErrPending) {
			t.Fatal(err)
		}
	}
	if *network != 0 || *calls != 0 || port.finishes != 0 || port.releases != 0 {
		t.Fatal("denied quote reached supplier")
	}
}
func TestLegacyExecutionMeteredForwardsOriginalRawWithoutGoKeyUsageOrPrice(t *testing.T) {
	m, port, input, _, calls := meteredExecutionFixture(t)
	got, err := m.LegacyExecuteRemote(context.Background(), meteredFixtureSession, "1", input)
	if err != nil || got.State != "done" || !got.SupplierReceiptAdmitted || got.Usage != nil || port.finishes != 1 || *calls != 1 {
		t.Fatalf("admitted supplier receipt not forwarded: %v %+v", err, got)
	}
	raw, err := base64.StdEncoding.DecodeString(port.lastFinish.RawResult)
	if err != nil || string(raw) != string(got.RawResult) || port.lastFinish.Usage != nil || port.lastFinish.ProofKind != LegacyProofOrdinary {
		t.Fatal("Go manufactured or replaced financial evidence")
	}
	expected, _ := LegacyMeteringResultDigest(raw)
	if expected != port.lastFinish.ResultDigest {
		t.Fatal("canonical receipt binding lost")
	}
	if _, err = m.LegacyExecuteRemote(context.Background(), meteredFixtureSession, "1", input); err != nil || *calls != 1 || port.reserves != 1 {
		t.Fatal("replay repeated supplier", err)
	}
}
func TestLegacyExecutionMeteredCoreRejectRetainsRawHoldAndNeverReexecutes(t *testing.T) {
	m, port, input, _, calls := meteredExecutionFixture(t)
	port.finishErr = ErrDenied
	got, err := m.LegacyExecuteRemote(context.Background(), meteredFixtureSession, "1", input)
	if !errors.Is(err, ErrPending) || got.Core.Held != 10 || len(got.RawResult) == 0 || got.Usage != nil || port.releases != 0 {
		t.Fatal("unverified receipt released or lost", err)
	}
	original := string(got.RawResult)
	m.now = func() time.Time { return time.Unix(500, 0) }
	for i := 0; i < 2; i++ {
		got, err = m.LegacyExecuteRemote(context.Background(), meteredFixtureSession, "1", input)
		if !errors.Is(err, ErrPending) || got.Core.Held != 10 || string(got.RawResult) != original {
			t.Fatal("rejected receipt changed outcome", err)
		}
	}
	if *calls != 1 || port.reserves != 1 || port.releases != 0 {
		t.Fatal("rejected raw receipt caused another execution/refund")
	}
}

func TestLegacyExecutionMeteredUnknownRetainsHoldAndNeverDispatchesAgain(t *testing.T) {
	m, port, input, _, calls := meteredExecutionFixture(t)
	upstream := m.net.client.Transport
	m.net.client.Transport = legacyProviderTransport(func(r *http.Request) (*http.Response, error) {
		raw, _ := io.ReadAll(r.Body)
		r.Body = io.NopCloser(bytes.NewReader(raw))
		var rpc struct {
			Method string `json:"method"`
		}
		_ = json.Unmarshal(raw, &rpc)
		if rpc.Method == "tools/call" {
			*calls++
			return nil, ErrUpstream
		}
		return upstream.RoundTrip(r)
	})
	got, err := m.LegacyExecuteRemote(context.Background(), meteredFixtureSession, "1", input)
	if !errors.Is(err, ErrPending) || got.Core.Held != 10 || got.ResultDigest != "" {
		t.Fatal("uncertain supplier outcome misclassified", err)
	}
	m.now = func() time.Time { return time.Unix(500, 0) }
	for i := 0; i < 2; i++ {
		if _, err = m.LegacyExecuteRemote(context.Background(), meteredFixtureSession, "1", input); !errors.Is(err, ErrPending) {
			t.Fatal(err)
		}
	}
	if *calls != 1 || port.reserves != 1 || port.finishes != 0 || port.releases != 0 {
		t.Fatal("UNKNOWN repeated execution or released hold")
	}
}
func TestLegacyExecutionMeteredExistingCoreCallACKNeverRestartsSupplier(t *testing.T) {
	m, port, input, network, calls := meteredExecutionFixture(t)
	port.reserveExisting = true
	got, err := m.LegacyExecuteRemote(context.Background(), meteredFixtureSession, "1", input)
	if !errors.Is(err, ErrPending) || got.Core.State != "running" || got.SupplierReceiptAdmitted || *network != 0 || *calls != 0 || port.finishes != 0 || port.releases != 0 {
		t.Fatal("old Core receipt restarted supplier", err)
	}
}

func TestLegacyExecutionMeteredFreeReportedUsageIsDisplayOnly(t *testing.T) {
	m, port, r := executionFixture(t)
	r.Core.Held = 0
	port.r.Held = 0
	_ = m.store.update(func(d *database) error { d.LegacyExecutions[r.ID] = r; return nil })
	tool := WebTool{BillingMode: "metered", BillingRules: []LegacyBillingRule{{Metric: "images", RateQuota: 2, MaxQuantity: 5}}}
	raw := json.RawMessage(`{"content":[{"type":"text","text":"ok"}],"usage":{"images":3}}`)
	got, err := m.legacyFinishRemote(context.Background(), port, r, tool, raw, "credential")
	if err != nil || got.Usage == nil || got.Usage.Source != "tool_reported" || got.Usage.Quantities["images"] != 3 || port.lastFinish.Usage != nil || port.finishes != 1 {
		t.Fatal("free display metadata became financial authority", err)
	}
}

func TestLegacyExecutionMeteredStaleRecoveryCannotClearOriginalAdmission(t *testing.T) {
	m, _, stale := executionFixture(t)
	if err := m.store.update(func(d *database) error {
		r := d.LegacyExecutions[stale.ID]
		r.SupplierReceiptAdmitted = true
		d.LegacyExecutions[r.ID] = r
		return nil
	}); err != nil {
		t.Fatal(err)
	}
	if err := m.legacySave(stale); err != nil {
		t.Fatal(err)
	}
	_ = m.store.view(func(d database) error {
		if !d.LegacyExecutions[stale.ID].SupplierReceiptAdmitted {
			t.Fatal("stale recovery cleared original Core admission")
		}
		return nil
	})
}
