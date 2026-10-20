// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package toolmarket

import (
	"context"
	"crypto/hmac"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"strconv"
	"strings"
	"testing"
	"time"
)

func TestLegacyExecutionResultBusinessStates(t *testing.T) {
	cases := []struct {
		raw              string
		success, unknown bool
	}{
		{`{"content":[{"type":"text","text":"ok"}]}`, true, false},
		{`{"isError":true,"content":[{"type":"text","text":"no"}]}`, false, false},
		{`{"content":[],"structuredContent":{"status":"running"}}`, false, true},
		{`{"content":[],"structuredContent":{"status":"failed"}}`, false, false},
		{`{"content":[]}`, false, false},
		{`{"content":[{"type":"text","text":"merchant-private"}]}`, false, false},
		{`{"inputRequests":[{}],"content":[]}`, false, true},
		{`{"content":[],"isError":false,"isError":true}`, false, false},
	}
	for _, c := range cases {
		raw, s, u, e := LegacyValidateRemoteResult(json.RawMessage(c.raw), "", "merchant-private")
		if e != nil || s != c.success || u != c.unknown {
			t.Fatalf("%s: success=%v unknown=%v err=%v", c.raw, s, u, e)
		}
		if string(raw) == c.raw && c.raw == cases[5].raw {
			t.Fatal("secret retained")
		}
	}
	_, success, _, e := LegacyValidateRemoteResult(json.RawMessage(`{"content":[],"structuredContent":{"n":"bad"}}`), `{"type":"object","properties":{"n":{"type":"integer"}},"required":["n"]}`)
	if e != nil || success {
		t.Fatal("schema mismatch charged")
	}
}

type executionCredits struct {
	reserveExisting                       bool
	reserveErr                            error
	lastFinish                            LegacyCreditFinish
	admitErr                              error
	witnessAvailable                      bool
	releaseID                             string
	releaseErr                            error
	r                                     LegacyCreditCall
	finishes, queries, reserves, releases int
	finishErr                             error
}

func (p *executionCredits) FixedWitnessAvailable() bool { return p.witnessAvailable }
func (p *executionCredits) AdmitFixedWitness(context.Context, string) error {
	if p.admitErr != nil {
		return p.admitErr
	}
	if !p.witnessAvailable {
		return ErrUnavailable
	}
	return nil
}
func (p *executionCredits) ReserveLegacyCredits(context.Context, string, LegacyCreditReserve) (LegacyCreditCall, error) {
	p.reserves++
	if p.reserveErr != nil {
		return LegacyCreditCall{}, p.reserveErr
	}
	r := p.r
	if p.reserveExisting {
		return r, nil
	}
	r.State = "reserved"
	r.DispatchID = ""
	r.DispatchClaimed = false
	return r, nil
}
func (p *executionCredits) DispatchLegacyCredits(context.Context, string, LegacyCreditID) (LegacyCreditCall, error) {
	return p.r, nil
}
func (p *executionCredits) FinishLegacyCredits(_ context.Context, in LegacyCreditFinish) (LegacyCreditCall, error) {
	p.finishes++
	p.lastFinish = in
	if p.finishErr != nil {
		return LegacyCreditCall{}, p.finishErr
	}
	if in.ResultDigest == "" {
		return LegacyCreditCall{}, ErrInvalid
	}
	p.r.State = "settled"
	if !in.Success {
		p.r.State = "released"
	}
	p.r.ResultDigest = in.ResultDigest
	return p.r, nil
}
func (p *executionCredits) QueryLegacyCredits(context.Context, string, LegacyCreditID) (LegacyCreditCall, error) {
	p.queries++
	return p.r, nil
}
func (p *executionCredits) UnknownLegacyCredits(context.Context, LegacyCreditID) (LegacyCreditCall, error) {
	p.r.State = "unknown"
	return p.r, nil
}
func (p *executionCredits) ReleaseLegacyCredits(_ context.Context, in LegacyCreditRelease) (LegacyCreditCall, error) {
	p.releaseID = in.CallID
	if p.releaseErr != nil {
		return LegacyCreditCall{}, p.releaseErr
	}
	p.releases++
	p.r.State = "released"
	return p.r, nil
}
func executionFixture(t *testing.T) (*Module, *executionCredits, LegacyExecution) {
	t.Helper()
	r := LegacyExecution{ID: "record", Owner: Principal{User: "1", Account: "1"}, Input: LegacyExecuteInput{ClientID: "web", RequestKey: "once", ToolID: "tool", VersionID: "version", GrantID: "grant"}, RequestHash: "request", InputDigest: "input", QuoteID: "quote", State: "unknown", ResolveBy: 200}
	c := LegacyCreditCall{ID: "call", Owner: r.Owner, ClientID: r.Input.ClientID, RequestKey: r.Input.RequestKey, ToolID: r.Input.ToolID, VersionID: r.Input.VersionID, GrantID: r.Input.GrantID, QuoteID: r.QuoteID, InputDigest: r.InputDigest, State: "running", Held: 10, DispatchID: "dispatch", ResolveBy: 200}
	r.Core = c
	d := emptyDB()
	d.LegacyExecutions = map[string]LegacyExecution{r.ID: r}
	m := &Module{store: &Store{data: d}, now: func() time.Time { return time.Unix(100, 0) }}
	return m, &executionCredits{r: c, witnessAvailable: true}, r
}
func TestLegacyExecutionPersistBeforeSettlementAndRecover(t *testing.T) {
	m, port, r := executionFixture(t)
	port.finishErr = ErrUnavailable
	got, e := m.legacyFinishRemote(context.Background(), port, r, WebTool{}, json.RawMessage(`{"content":[{"type":"text","text":"ok"}]}`), "credential")
	if !errors.Is(e, ErrPending) || got.ResultDigest == "" || port.finishes != 1 {
		t.Fatalf("result not retained: %+v %v", got, e)
	}
	var saved LegacyExecution
	_ = m.store.view(func(d database) error { saved = d.LegacyExecutions[r.ID]; return nil })
	if saved.ResultDigest == "" || saved.State != "result" {
		t.Fatal("financial request preceded persistence")
	}
	port.finishErr = nil
	done, e := m.legacyRecoverExecution(context.Background(), port, "credential", saved)
	if e != nil || done.State != "done" || port.finishes != 2 || port.reserves != 0 {
		t.Fatalf("recovery failed: %+v %v", done, e)
	}
	changed := done
	changed.ResultDigest = "different"
	if !errors.Is(m.legacySave(changed), ErrConflict) {
		t.Fatal("immutable result overwritten")
	}
}
func TestLegacyExecutionUnknownNeverDispatchesOrCharges(t *testing.T) {
	m, port, r := executionFixture(t)
	port.r.State = "unknown"
	for i := 0; i < 3; i++ {
		_, e := m.legacyRecoverExecution(context.Background(), port, "credential", r)
		if !errors.Is(e, ErrPending) {
			t.Fatal(e)
		}
	}
	if port.finishes != 0 || port.reserves != 0 || port.queries != 3 {
		t.Fatalf("unknown retried execution: %+v", port)
	}
}
func TestLegacyExecutionFailureWithoutCoreRawProofPreservesPaidHold(t *testing.T) {
	m, port, r := executionFixture(t)
	port.witnessAvailable = false
	got, e := m.legacyFinishRemote(context.Background(), port, r, WebTool{}, json.RawMessage(`{"isError":true,"content":[{"type":"text","text":"failed"}]}`), "credential")
	if !errors.Is(e, ErrPending) || got.Success || got.Core.State != "running" || got.Core.Held != 10 || port.finishes != 0 || port.releases != 0 || got.ResultDigest == "" {
		t.Fatalf("host failure assertion moved paid funds: %+v %v", got, e)
	}
}
func TestLegacyExecutionFreeFailureAndResultExpires(t *testing.T) {
	m, port, r := executionFixture(t)
	r.Core.Held = 0
	port.r.Held = 0
	_ = m.store.update(func(d *database) error { d.LegacyExecutions[r.ID] = r; return nil })
	got, e := m.legacyFinishRemote(context.Background(), port, r, WebTool{}, json.RawMessage(`{"isError":true,"content":[{"type":"text","text":"failed"}]}`), "credential")
	if e != nil || got.Success || got.Core.State != "released" {
		t.Fatalf("free failure: %+v %v", got, e)
	}
	m.now = func() time.Time { return time.Unix(4000, 0) }
	got, e = m.legacyRecoverExecution(context.Background(), port, "credential", got)
	if e != nil || got.Result != nil {
		t.Fatal("expired delivery returned", e)
	}
}

type executionAuthority struct {
	webTestAuthority
	executionCredits
}

func TestLegacyExecutionRemoteWireOnceReplayAndChangedInput(t *testing.T) {
	tool := Tool{Name: "echo", Description: "echo", InputSchema: json.RawMessage(`{"type":"object"}`)}
	authority := &executionAuthority{}
	d := emptyDB()
	w := webState(&d)
	w.Services["service"] = WebServiceRecord{ID: "service", OwnerID: 1, LiveVersionID: "version", Status: "published"}
	w.Versions["version"] = webSnapshot{Version: WebVersion{ID: "version", ServiceID: "service", Status: "published", ExecutionType: "remote", Visibility: "public", Endpoint: "https://tools.example.test/mcp"}, Digest: "snapshot", RemoteDigests: map[string]string{"tool": digest(tool)}, Tools: []WebTool{{ToolID: "tool", VersionID: "version", Name: "echo", InputSchema: `{"type":"object"}`}}}
	w.Installations[digest([]any{int64(1), "web", "tool"})] = WebInstallation{UserID: 1, ClientID: "web", ToolID: "tool", VersionID: "version"}
	access := legacyAccess(&d)
	access.Grants["grant"] = LegacyConsentGrant{ID: "grant", UserID: 1, ClientID: "web", ToolID: "tool", VersionID: "version", QuoteID: "quote", ExpiresAt: 1000}
	s := &Store{data: d}
	m, e := New(s, authority, Config{WebOnly: true})
	if e != nil {
		t.Fatal(e)
	}
	m.now = func() time.Time { return time.Unix(100, 0) }
	input := LegacyExecuteInput{ClientID: "web", RequestKey: "once", ToolID: "tool", VersionID: "version", GrantID: "grant", Arguments: json.RawMessage(`{}`)}
	priceDigest, _ := LegacyQuotePricingDigest(WebTool{}.LegacyPricing())
	authority.r = LegacyCreditCall{PricingDigest: priceDigest, ID: "call", Owner: Principal{User: "1", Account: "1"}, ClientID: "web", RequestKey: "once", QuoteID: "quote", GrantID: "grant", ToolID: "tool", VersionID: "version", InputDigest: digest(map[string]any{}), ResolveBy: 220, Held: 0, State: "running", DispatchID: "dispatch", DispatchClaimed: true}
	calls := 0
	discovery := 0
	m.net.client = &http.Client{Transport: legacyProviderTransport(func(r *http.Request) (*http.Response, error) {
		var req struct {
			ID     json.RawMessage `json:"id"`
			Method string          `json:"method"`
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
			discovery++
			result = map[string]any{"tools": []Tool{tool}}
		case "tools/call":
			calls++
			result = map[string]any{"content": []any{map[string]any{"type": "text", "text": "ok"}}}
		default:
			t.Fatal(req.Method)
		}
		raw, _ := json.Marshal(map[string]any{"jsonrpc": "2.0", "id": req.ID, "result": result})
		return &http.Response{StatusCode: 200, Header: http.Header{"Content-Type": []string{"application/json"}}, Body: io.NopCloser(strings.NewReader(string(raw)))}, nil
	})}
	for _, kind := range []string{"fixed", "fixed_quote_denied", "provider"} {
		_ = m.store.update(func(d *database) error {
			v := d.Web.Versions["version"]
			if strings.HasPrefix(kind, "fixed") {
				v.Tools[0].PriceQuota = 10
			} else if kind == "metered" {
				v.Tools[0].BillingMode = "metered"
				v.Tools[0].BillingRules = []LegacyBillingRule{{Metric: "images", RateQuota: 2, MaxQuantity: 5}}
				v.Tools[0].PriceQuota = 10
			} else {
				v.Tools[0].ProviderPricing = &LegacyProviderPricing{Provider: "monid", Multiplier: "2"}
			}
			d.Web.Versions["version"] = v
			return nil
		})
		wantError := ErrUnavailable
		if kind == "fixed_quote_denied" {
			authority.witnessAvailable = true
			authority.admitErr = ErrDenied
			wantError = ErrDenied
		}
		if _, err := m.LegacyExecuteRemote(context.Background(), "1", "1", input); !errors.Is(err, wantError) || calls != 0 || discovery != 0 || authority.reserves != 0 {
			t.Fatalf("paid %s used unverified receipt authority: err=%v calls=%d discovery=%d reserve=%d", kind, err, calls, discovery, authority.reserves)
		}
		authority.witnessAvailable = false
		authority.admitErr = nil
		_ = m.store.update(func(d *database) error {
			v := d.Web.Versions["version"]
			v.Tools[0] = WebTool{ToolID: "tool", VersionID: "version", Name: "echo", InputSchema: `{"type":"object"}`}
			d.Web.Versions["version"] = v
			return nil
		})
	}
	got, e := m.LegacyExecuteRemote(context.Background(), "1", "1", input)
	if e != nil || got.State != "done" || calls != 1 {
		t.Fatalf("execution: %+v err=%v calls=%d", got, e, calls)
	}
	got, e = m.LegacyExecuteRemote(context.Background(), "1", "1", input)
	if e != nil || got.State != "done" || calls != 1 || discovery != 1 {
		t.Fatalf("replay re-executed: %+v %v %d/%d", got, e, calls, discovery)
	}
	input.Arguments = json.RawMessage(`{"changed":true}`)
	if _, e = m.LegacyExecuteRemote(context.Background(), "1", "1", input); !errors.Is(e, ErrConflict) {
		t.Fatal("changed idempotency accepted", e)
	}
	if calls != 1 || authority.reserves != 1 {
		t.Fatalf("multiple side effects %d/%d", calls, authority.reserves)
	}
}
func TestLegacyExecutionDeadlinePreservesRunningUnknownAndLostDispatchACK(t *testing.T) {
	cases := []struct{ name, local, core, dispatch string }{
		{"running", "unknown", "running", "dispatch"},
		{"unknown", "unknown", "unknown", "dispatch"},
		{"dispatch_ack_lost", "unknown", "reserved", ""},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			m, port, r := executionFixture(t)
			r.State = c.local
			r.Core.State = c.core
			r.Core.DispatchID = c.dispatch
			port.r = r.Core
			_ = m.store.update(func(d *database) error { d.LegacyExecutions[r.ID] = r; return nil })
			authority := &executionAuthority{executionCredits: *port}
			m.authority = authority
			m.now = func() time.Time { return time.Unix(201, 0) }
			count, e := m.LegacyRecoverPending(context.Background(), 10)
			if e != nil || count != 0 || authority.releases != 0 || authority.finishes != 0 || authority.reserves != 0 {
				t.Fatal("deadline invented release/settlement", count, e, authority.executionCredits)
			}
			got, e := m.legacyRecoverExecution(context.Background(), authority, "credential", r)
			if !errors.Is(e, ErrPending) || got.Core.Held != 10 || got.Core.State != c.core || got.State != "unknown" || authority.releases != 0 {
				t.Fatalf("query/replay released uncertain hold: %+v %v", got, e)
			}
		})
	}
}
func TestLegacyExecutionDeadlineOnlyAbortsKnownUndispatchedReservation(t *testing.T) {
	m, port, r := executionFixture(t)
	r.State = "reserved"
	r.Core.State = "reserved"
	r.Core.DispatchID = ""
	port.r = r.Core
	_ = m.store.update(func(d *database) error { d.LegacyExecutions[r.ID] = r; return nil })
	authority := &executionAuthority{executionCredits: *port}
	m.authority = authority
	m.now = func() time.Time { return time.Unix(201, 0) }
	count, e := m.LegacyRecoverPending(context.Background(), 10)
	if e != nil || count != 1 || authority.releases != 1 || authority.releaseID != r.Core.ID || authority.finishes != 0 {
		t.Fatal("known undispatched abort failed", count, e, authority.executionCredits)
	}
	_ = m.store.view(func(d database) error {
		got := d.LegacyExecutions[r.ID]
		if got.State != "done" || got.Core.State != "released" {
			t.Fatal(got)
		}
		return nil
	})
}

func TestLegacyExecutionMeteredUsageAndMissingEvidence(t *testing.T) {
	for _, valid := range []bool{true, false} {
		t.Run(strconv.FormatBool(valid), func(t *testing.T) {
			m, port, r := executionFixture(t)
			r.Core.PricingDigest = "core-price"
			port.r.PricingDigest = "core-price"
			_ = m.store.update(func(d *database) error { d.LegacyExecutions[r.ID] = r; return nil })
			tool := WebTool{BillingMode: "metered", BillingRules: []LegacyBillingRule{{Metric: "images", RateQuota: 2, MaxQuantity: 5}}}
			raw := json.RawMessage(`{"content":[{"type":"text","text":"ok"}],"usage":{"images":3}}`)
			if !valid {
				raw = json.RawMessage(`{"content":[{"type":"text","text":"ok"}]}`)
			}
			got, e := m.legacyFinishRemote(context.Background(), port, r, tool, raw, "credential")
			if valid {
				if !errors.Is(e, ErrUnavailable) || port.finishes != 0 {
					t.Fatal("typed usage became authority", e, port.finishes)
				}
				if got.Usage != nil || string(got.RawResult) != string(raw) || got.Core.State != "running" {
					t.Fatal(got)
				}
			} else if !errors.Is(e, ErrUnavailable) || port.finishes != 0 || got.Usage != nil || string(got.RawResult) != string(raw) || got.Core.State != "running" {
				t.Fatal("missing raw receipt released or billed", got, e)
			}
		})
	}
}

type executionMeteringResolver struct {
	binding LegacyMeteringBinding
	config  LegacyMeteringConfig
}

func (p executionMeteringResolver) ResolveLegacyMetering(_ context.Context, b LegacyMeteringBinding) (*LegacyMeteringConfig, error) {
	if b != p.binding {
		return nil, ErrDenied
	}
	copy := p.config
	return &copy, nil
}
func TestLegacyExecutionSignedUsagePreservesSupplierAndCoreDigests(t *testing.T) {
	m, port, r := executionFixture(t)
	tool := WebTool{BillingMode: "metered", BillingRules: []LegacyBillingRule{{Metric: "input_tokens", RateQuota: 100, MaxQuantity: 100}}}
	price := tool.LegacyPricing()
	supplierDigest, _ := LegacyMeteringPricingDigest(price)
	coreDigest, _ := LegacyQuotePricingDigest(price)
	if supplierDigest == coreDigest {
		t.Fatal("distinct versioned pricing encodings collapsed")
	}
	r.Core.PricingDigest = coreDigest
	port.r.PricingDigest = coreDigest
	r.MeteringBinding = LegacyMeteringBinding{ServiceID: "service", VersionID: "version", Endpoint: "https://example.test/mcp", ToolID: "tool", RemoteDigest: "remote"}
	cfg := LegacyMeteringConfig{AdapterID: "trusted-adapter", Key: []byte(strings.Repeat("k", 32)), Metrics: []string{"input_tokens"}}
	m.legacyMetering = executionMeteringResolver{r.MeteringBinding, cfg}
	_ = m.store.update(func(d *database) error { d.LegacyExecutions[r.ID] = r; return nil })
	base := json.RawMessage(`{"content":[{"type":"text","text":"ok"}],"usage":{"input_tokens":3}}`)
	resultDigest, _ := LegacyMeteringResultDigest(base)
	receipt := legacyMeteringReceipt{AdapterID: cfg.AdapterID, Context: LegacyMeteringContext{CallID: r.Core.ID, VersionID: r.Input.VersionID, ToolID: r.Input.ToolID, InputDigest: r.InputDigest, PricingDigest: supplierDigest}, Quantities: map[string]int64{"input_tokens": 3}, ResultDigest: resultDigest}
	message, _ := json.Marshal(struct {
		AdapterID    string                `json:"adapter_id"`
		Context      LegacyMeteringContext `json:"context"`
		Quantities   map[string]int64      `json:"quantities"`
		ResultDigest string                `json:"result_digest"`
	}{receipt.AdapterID, receipt.Context, receipt.Quantities, receipt.ResultDigest})
	mac := hmac.New(sha256.New, cfg.Key)
	mac.Write([]byte("lmm-tool-market-metering-v1\n"))
	mac.Write(message)
	receipt.Signature = hex.EncodeToString(mac.Sum(nil))
	var value map[string]any
	_ = json.Unmarshal(base, &value)
	value["_meta"] = map[string]any{"lmm_metering": receipt}
	raw, _ := json.Marshal(value)
	got, e := m.legacyFinishRemote(context.Background(), port, r, tool, raw, "credential")
	if !errors.Is(e, ErrUnavailable) || port.finishes != 0 || got.Usage != nil || string(got.RawResult) != string(raw) || got.AuthorityResultDigest != resultDigest || got.Core.State != "running" {
		t.Fatalf("signed evidence incorrectly bound %+v %v", got, e)
	}
}

func TestLegacyExecutionLostReserveACKRemainsUnresolvedWithoutCoreReceipt(t *testing.T) {
	for _, exists := range []bool{true, false} {
		t.Run(strconv.FormatBool(exists), func(t *testing.T) {
			m, port, r := executionFixture(t)
			r.Core = LegacyCreditCall{}
			r.State = "prepared"
			_ = m.store.update(func(d *database) error { d.LegacyExecutions[r.ID] = r; return nil })
			if !exists {
				port.releaseErr = ErrMissing
			}
			authority := &executionAuthority{executionCredits: *port}
			m.authority = authority
			m.now = func() time.Time { return time.Unix(201, 0) }
			count, e := m.LegacyRecoverPending(context.Background(), 10)
			if e != nil || count != 0 || authority.releases != 0 || authority.reserves != 0 || authority.finishes != 0 {
				t.Fatal("missing receipt invented financial state", count, e, authority.executionCredits)
			}
			_ = m.store.view(func(d database) error {
				got := d.LegacyExecutions[r.ID]
				if got.State != "prepared" || got.Core.ID != "" {
					t.Fatal("invented Core receipt or release", got)
				}
				return nil
			})
		})
	}
}

func TestLegacyExecutionWitnessRoutingDerivesFromDurableFixedEconomics(t *testing.T) {
	_, _, base := executionFixture(t)
	base.Success = true
	base.RawResult = []byte(`{"content":[]}`)
	base.AuthorityResultDigest = "original-proof-digest"
	if got := legacyFinishRequest(base); got.ProofKind != LegacyProofFixedWitness {
		t.Fatal("positive fixed lost witness routing", got.ProofKind)
	}
	cases := []LegacyExecution{base, base, base, base}
	cases[0].Core.Held = 0
	cases[1].Pricing.BillingMode = "metered"
	cases[2].ProviderPricing = &LegacyProviderPricing{Provider: "monid", Multiplier: "2"}
	cases[3].RawResult = nil
	cases[3].AuthorityResultDigest = ""
	for _, r := range cases {
		request := legacyFinishRequest(r)
		if request.ProofKind != LegacyProofOrdinary {
			t.Fatal("non-fixed/non-success gained purpose", request.ProofKind)
		}
		raw, _ := json.Marshal(request)
		if strings.Contains(string(raw), "ProofKind") || strings.Contains(string(raw), "proof_kind") {
			t.Fatal("proof kind serialized", string(raw))
		}
	}
}
