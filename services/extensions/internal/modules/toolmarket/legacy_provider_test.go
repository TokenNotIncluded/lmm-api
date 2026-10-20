// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package toolmarket

import (
	"context"
	"encoding/json"
	"io"
	"net/http"
	"strings"
	"testing"
)

func TestLegacyProviderCapsMatchFixedMain(t *testing.T) {
	for _, s := range []string{"1000000", "1e6", "999999.999999999999"} {
		if _, err := legacyProviderDecimal(s); err != nil {
			t.Fatal(s, err)
		}
	}
	if _, err := legacyProviderDecimal("1000000.000000000001"); err == nil {
		t.Fatal("main decimal cap bypass")
	}
	p, err := NormalizeLegacyPricing(LegacyPricing{BillingMode: "input_tokens", InputTokenPriceQuota: 1, MaxInputTokens: 1000000})
	if err != nil || p.PriceQuota != 1 {
		t.Fatal(p, err)
	}
	p, err = NormalizeLegacyPricing(LegacyPricing{BillingMode: "metered", BillingRules: []LegacyBillingRule{{"input_tokens", 1, 1000000000000}}})
	if err != nil || p.PriceQuota != 1000000 {
		t.Fatal(p, err)
	}
	if _, err := NormalizeLegacyPricing(LegacyPricing{BillingMode: "metered", BillingRules: []LegacyBillingRule{{"input_tokens", 1, 1000000000001}}}); err == nil {
		t.Fatal("main quantity cap bypass")
	}
}
func TestLegacyProviderHostAndAccountBoundaries(t *testing.T) {
	for _, preset := range LegacyProviderPresets() {
		p := LegacyProviderPricing{preset.ID, "2"}
		if err := ValidateLegacyProviderTool(preset.Endpoint, preset.ExecuteTool, &p); err != nil {
			t.Fatal(err)
		}
		if err := ValidateLegacyProviderTool(preset.Endpoint, preset.ExecuteTool, nil); err == nil {
			t.Fatal("unpriced execute")
		}
		for _, tool := range preset.ReadTools {
			if err := ValidateLegacyProviderTool(preset.Endpoint, tool, nil); err != nil {
				t.Fatal(err)
			}
		}
		for _, endpoint := range []string{preset.Endpoint + "/other", strings.Replace(preset.Endpoint, "/v1", ":443/v1", 1), strings.Replace(preset.Endpoint, "https://", "https://user@", 1), strings.ToUpper(preset.Endpoint)} {
			if err := ValidateLegacyProviderTool(endpoint, preset.InspectTool, nil); err == nil {
				t.Fatal("endpoint alias", endpoint)
			}
		}
		if err := ValidateLegacyProviderTool(preset.Endpoint, "account_manage", nil); err == nil {
			t.Fatal("account tool bypass")
		}
	}
	presets := LegacyProviderPresets()
	presets[0].ReadTools[0] = "account_delete"
	if LegacyProviderPresets()[0].ReadTools[0] == "account_delete" {
		t.Fatal("mutable registry")
	}
	for _, name := range []string{"agentkey/account", "AgentKey/account", "agentkey_account", "no-slash", "vendor/tool\n", " vendor/tool"} {
		if _, err := LegacyProviderQuoteArguments("agentkey", map[string]any{"name": name}); err == nil {
			t.Fatal(name)
		}
	}
	query, err := LegacyProviderQuoteArguments("agentkey", map[string]any{"name": "vendor/image", "params": map[string]any{"private": "ignored"}, "account": "ignored"})
	if err != nil || len(query) != 1 || query["name"] != "vendor/image" {
		t.Fatal(query, err)
	}
	if _, err := LegacyProviderQuoteArguments("monid", map[string]any{"provider": "vendor", "endpoint": "//evil.test"}); err == nil {
		t.Fatal("arbitrary URL")
	}
}
func TestLegacyProviderQuoteParsersExact(t *testing.T) {
	q, err := ParseLegacyAgentKeyQuote(json.RawMessage(`{"usd_per_call":0.000000000000000001}`))
	if err != nil || q.AmountUSD != "0.000000000000000001" {
		t.Fatal(q, err)
	}
	q, err = ParseLegacyMonidQuote(json.RawMessage(`{"type":"PER_CALL","amount":{"value":"0.000001","currency":"USD"},"flatFee":0.000001}`))
	if err != nil || q.AmountUSD != "0.000001" || q.FlatFeeUSD != "0.000001" {
		t.Fatal(q, err)
	}
	for _, raw := range []string{`{"type":"PER_CALL","amount":1,"currency":"EUR"}`, `{"type":"PER_CALL","amount":{"value":1,"currency":"EUR"}}`, `{"type":"SUBSCRIPTION","amount":1,"currency":"USD"}`, `{"type":"PER_CALL","amount":1,"amount":2,"currency":"USD"}`} {
		if _, err := ParseLegacyMonidQuote(json.RawMessage(raw)); err == nil {
			t.Fatal(raw)
		}
	}
	q, err = ParseLegacyMonidQuote(json.RawMessage(`{"type":"PER_RESULT","amount":1,"currency":"USD"}`))
	if err != nil || q.Unit != "result" {
		t.Fatal(q, err)
	}
	if _, err := LegacyProviderCharge(q, "2", MaxCreditAmount, 0); err == nil {
		t.Fatal("variable charge")
	}
}

type legacyProviderTransport func(*http.Request) (*http.Response, error)

func (f legacyProviderTransport) RoundTrip(r *http.Request) (*http.Response, error) { return f(r) }

func TestLegacyProviderDiscoveryUsesSafeSessionInspectOnly(t *testing.T) {
	preset, _ := legacyProviderPreset("agentkey")
	calls := 0
	transport := legacyProviderTransport(func(r *http.Request) (*http.Response, error) {
		calls++
		if r.URL.String() != preset.Endpoint || r.Method != "POST" {
			t.Fatal("wrong destination", r.URL)
		}
		var request struct {
			ID     json.RawMessage `json:"id"`
			Method string          `json:"method"`
			Params struct {
				Name      string         `json:"name"`
				Arguments map[string]any `json:"arguments"`
			} `json:"params"`
		}
		body, _ := io.ReadAll(r.Body)
		if err := json.Unmarshal(body, &request); err != nil {
			t.Fatal(err)
		}
		if request.Method != "tools/call" || request.Params.Name != "describe_tool" || len(request.Params.Arguments) != 1 || request.Params.Arguments["name"] != "vendor/image" {
			t.Fatal(string(body))
		}
		result := json.RawMessage(`{"structuredContent":{"name":"VENDOR/IMAGE","cost":{"usd_per_call":"0.002"},"params":{"type":"object","properties":{"n":{"type":"integer","minimum":1,"maximum":4}},"required":["n"],"additionalProperties":false}}}`)
		response, _ := json.Marshal(map[string]any{"jsonrpc": "2.0", "id": request.ID, "result": result})
		return &http.Response{StatusCode: 200, Header: http.Header{"Content-Type": []string{"application/json"}}, Body: io.NopCloser(strings.NewReader(string(response)))}, nil
	})
	session := &mcpSession{net: &network{client: &http.Client{Transport: transport}}, endpoint: preset.Endpoint, headers: http.Header{}}
	q, err := DiscoverLegacyProviderQuote(context.Background(), LegacyProviderPricing{"agentkey", "2"}, preset.Endpoint, preset.ExecuteTool, json.RawMessage(`{"name":"vendor/image","params":{"n":2},"account":"do-not-forward"}`), session.legacyProviderInspector(), validateArgs)
	if err != nil || q.AmountUSD != "0.002" || calls != 1 {
		t.Fatal(q, err, calls)
	}
	if _, err := session.legacyProviderInspector()(context.Background(), LegacyProviderInspection{preset.Endpoint, "execute_tool", map[string]any{}}); err == nil || calls != 1 {
		t.Fatal("inspector executed operation")
	}
	if _, err := session.legacyProviderInspector()(context.Background(), LegacyProviderInspection{"https://evil.test/mcp", preset.InspectTool, map[string]any{}}); err == nil || calls != 1 {
		t.Fatal("inspector changed endpoint")
	}
}
func TestLegacyProviderDiscoveryRejectsBadEvidence(t *testing.T) {
	preset, _ := legacyProviderPreset("monid")
	good := `{"structuredContent":{"provider":"vendor","endpoint":"/image","price":{"type":"PER_CALL","amount":"0.002","currency":"USD"},"inputSchema":{"type":"object","properties":{"n":{"type":"integer","minimum":1}},"required":["n"],"additionalProperties":false}}}`
	inputs := json.RawMessage(`{"provider":"vendor","endpoint":"/image","input":{"n":2}}`)
	call := func(raw string) LegacyProviderInspectFunc {
		return func(_ context.Context, in LegacyProviderInspection) (json.RawMessage, error) {
			if in.Tool != "monid_inspect" || len(in.Arguments) != 2 {
				t.Fatal(in)
			}
			return json.RawMessage(raw), nil
		}
	}
	p := LegacyProviderPricing{"monid", "2"}
	q, err := DiscoverLegacyProviderQuote(context.Background(), p, preset.Endpoint, preset.ExecuteTool, inputs, call(good), validateArgs)
	if err != nil || q.TargetProvider != "vendor" || q.TargetEndpoint != "/image" {
		t.Fatal(q, err)
	}
	for _, raw := range []string{
		strings.Replace(good, `"provider":"vendor"`, `"provider":"other"`, 1),
		strings.Replace(good, `"PER_CALL"`, `"PER_RESULT"`, 1),
		strings.Replace(good, `"inputSchema":`, `"missingSchema":`, 1),
		strings.Replace(good, `{"structuredContent":`, `{"isError":true,"structuredContent":`, 1),
		strings.Replace(good, `{"structuredContent":`, `{"requestState":"confirm","structuredContent":`, 1),
		strings.Replace(good, `{"structuredContent":`, `{"inputRequests":[{}],"structuredContent":`, 1),
		strings.Replace(good, `"price":`, `"credential":"merchant-secret","price":`, 1),
		strings.Replace(good, `"inputSchema":`, `"schema":{"input":{"type":"object"}},"inputSchema":`, 1),
	} {
		if _, err := DiscoverLegacyProviderQuote(context.Background(), p, preset.Endpoint, preset.ExecuteTool, inputs, call(raw), validateArgs, "merchant-secret"); err == nil {
			t.Fatal("bad evidence accepted", raw)
		}
	}
	if _, err := DiscoverLegacyProviderQuote(context.Background(), p, preset.Endpoint, preset.ExecuteTool, json.RawMessage(`{"provider":"vendor","endpoint":"/image","input":{"n":0}}`), call(good), validateArgs); err == nil {
		t.Fatal("selected-operation schema bypass")
	}
	if _, err := DiscoverLegacyProviderQuote(context.Background(), p, preset.Endpoint, preset.ExecuteTool, inputs, call(good), nil); err == nil {
		t.Fatal("missing schema validator")
	}
}
