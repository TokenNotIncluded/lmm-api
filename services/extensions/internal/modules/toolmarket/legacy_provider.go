// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
// Adapted from fixed main 3948a0a internal/marketprovider/pricing.go and
// service/tool_market_provider.go. No financial authority or execution calls.
package toolmarket

import (
	"context"
	"encoding/json"
	"math/big"
	"net/url"
	"slices"
	"strings"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/redaction"
)

type LegacyProviderPreset struct {
	ReadTools     []string `json:"read_tools"`
	ID            string   `json:"id"`
	Name          string   `json:"name"`
	Endpoint      string   `json:"endpoint"`
	ExecuteTool   string   `json:"execute_tool"`
	InspectTool   string   `json:"inspect_tool"`
	Documentation string   `json:"documentation"`
	OAuth         bool     `json:"oauth"`
}

func LegacyProviderPresets() []LegacyProviderPreset {
	return []LegacyProviderPreset{
		{[]string{"monid_discover", "monid_inspect"}, "monid", "Monid", "https://mcp.monid.ai/v1", "monid_run", "monid_inspect", "https://docs.monid.ai", true},
		{[]string{"find_tools", "list_tools", "describe_tool"}, "agentkey", "AgentKey", "https://api.agentkey.app/v1/mcp", "execute_tool", "describe_tool", "https://docs.agentkey.app", true},
	}
}
func legacyProviderPreset(id string) (LegacyProviderPreset, bool) {
	for _, p := range LegacyProviderPresets() {
		if p.ID == id {
			return p, true
		}
	}
	return LegacyProviderPreset{}, false
}

type LegacyProviderPricing struct {
	Provider   string `json:"provider"`
	Multiplier string `json:"multiplier"`
}

func (p LegacyProviderPricing) Validate(endpoint, tool string) error {
	preset, ok := legacyProviderPreset(p.Provider)
	if !ok || endpoint != preset.Endpoint || tool != preset.ExecuteTool {
		return ErrPrice
	}
	m, err := legacyProviderDecimal(p.Multiplier)
	if err != nil || m.Cmp(big.NewRat(1, 1)) < 0 || m.Cmp(big.NewRat(100, 1)) > 0 {
		return ErrPrice
	}
	return nil
}

// Known provider hosts cannot bypass their account/pricing boundary through a
// different path, port, casing, trailing dot or an unpriced execute tool.
func ValidateLegacyProviderTool(endpoint, name string, pricing *LegacyProviderPricing) error {
	u, err := url.Parse(endpoint)
	if err != nil {
		return ErrPrice
	}
	for _, p := range LegacyProviderPresets() {
		official, _ := url.Parse(p.Endpoint)
		if !strings.EqualFold(strings.TrimSuffix(u.Hostname(), "."), official.Hostname()) {
			continue
		}
		if endpoint != p.Endpoint {
			return ErrPrice
		}
		if name == p.ExecuteTool {
			if pricing == nil {
				return ErrPrice
			}
			return pricing.Validate(endpoint, name)
		}
		if pricing != nil || !slices.Contains(p.ReadTools, name) {
			return ErrPrice
		}
		return nil
	}
	if pricing != nil {
		return pricing.Validate(endpoint, name)
	}
	return nil
}
func LegacyProviderQuoteArguments(provider string, arguments map[string]any) (map[string]any, error) {
	text := func(key string) (string, error) {
		s, ok := arguments[key].(string)
		if !ok || len(s) == 0 || len(s) > 256 || strings.TrimSpace(s) != s || strings.ContainsAny(s, "\r\n\x00") {
			return "", ErrPrice
		}
		return s, nil
	}
	switch provider {
	case "agentkey":
		name, err := text("name")
		if err != nil || !strings.Contains(name, "/") || strings.EqualFold(name, "agentkey_account") || strings.HasPrefix(strings.ToLower(name), "agentkey/") {
			return nil, ErrPrice
		}
		return map[string]any{"name": name}, nil
	case "monid":
		provider, err := text("provider")
		if err != nil {
			return nil, err
		}
		endpoint, err := text("endpoint")
		if err != nil || !strings.HasPrefix(endpoint, "/") || strings.HasPrefix(endpoint, "//") {
			return nil, ErrPrice
		}
		return map[string]any{"provider": provider, "endpoint": endpoint}, nil
	default:
		return nil, ErrPrice
	}
}

type LegacyProviderInspection struct {
	Endpoint, Tool string
	Arguments      map[string]any
}

// The caller owns authentication, credential pinning, context deadline and the
// network-safe MCP connection. The leaf issues exactly one read-only inspect
// call and never runs a provider operation, registers an account or spends money.
type LegacyProviderInspectFunc func(context.Context, LegacyProviderInspection) (json.RawMessage, error)
type LegacyProviderSchemaValidator func(schema, input json.RawMessage) error

// legacyProviderInspector reuses the existing DNS-pinned/no-proxy MCP transport
// after the owner has authenticated/connected it. It cannot inspect another URL.
func (c *mcpSession) legacyProviderInspector() LegacyProviderInspectFunc {
	return func(ctx context.Context, in LegacyProviderInspection) (json.RawMessage, error) {
		if c == nil || c.endpoint != in.Endpoint {
			return nil, ErrDenied
		}
		allowed := false
		for _, p := range LegacyProviderPresets() {
			if p.Endpoint == in.Endpoint && p.InspectTool == in.Tool {
				allowed = true
			}
		}
		if !allowed {
			return nil, ErrDenied
		}
		return c.rpc(ctx, "tools/call", map[string]any{"name": in.Tool, "arguments": in.Arguments})
	}
}
func DiscoverLegacyProviderQuote(ctx context.Context, pricing LegacyProviderPricing, endpoint, tool string, arguments json.RawMessage, inspect LegacyProviderInspectFunc, validate LegacyProviderSchemaValidator, secrets ...string) (LegacyProviderQuote, error) {
	if pricing.Validate(endpoint, tool) != nil || inspect == nil || validate == nil {
		return LegacyProviderQuote{}, ErrPrice
	}
	var args map[string]any
	if len(arguments) > 128<<10 || strictLegacyJSON(arguments, &args, false) != nil || args == nil {
		return LegacyProviderQuote{}, ErrInvalid
	}
	query, err := LegacyProviderQuoteArguments(pricing.Provider, args)
	if err != nil {
		return LegacyProviderQuote{}, err
	}
	preset, _ := legacyProviderPreset(pricing.Provider)
	raw, err := inspect(ctx, LegacyProviderInspection{preset.Endpoint, preset.InspectTool, query})
	if err != nil {
		return LegacyProviderQuote{}, err
	}
	if len(raw) > maxBody {
		return LegacyProviderQuote{}, ErrPrice
	}
	var exact map[string]any
	if strictLegacyJSON(raw, &exact, false) != nil || exact == nil {
		return LegacyProviderQuote{}, ErrPrice
	}
	// Reject reflected secrets instead of persisting a sanitized but false quote.
	safe, err := redaction.JSON(raw, secrets...)
	if err != nil {
		return LegacyProviderQuote{}, ErrPrice
	}
	var safeExact any
	if strictLegacyJSON(safe, &safeExact, false) != nil || digest(safeExact) != digest(exact) {
		return LegacyProviderQuote{}, ErrPrice
	}
	if failed, _ := exact["isError"].(bool); failed {
		return LegacyProviderQuote{}, ErrPrice
	}
	if state, ok := exact["requestState"]; ok && state != "" && state != nil {
		return LegacyProviderQuote{}, ErrPrice
	}
	if requests, ok := exact["inputRequests"]; ok && requests != nil {
		switch v := requests.(type) {
		case []any:
			if len(v) > 0 {
				return LegacyProviderQuote{}, ErrPrice
			}
		case map[string]any:
			if len(v) > 0 {
				return LegacyProviderQuote{}, ErrPrice
			}
		default:
			return LegacyProviderQuote{}, ErrPrice
		}
	}
	data, err := legacyProviderPayload(exact)
	if err != nil {
		return LegacyProviderQuote{}, err
	}
	var q LegacyProviderQuote
	var schema, input any
	if pricing.Provider == "agentkey" {
		name, _ := data["name"].(string)
		if !strings.EqualFold(name, query["name"].(string)) {
			return q, ErrPrice
		}
		cost, _ := json.Marshal(data["cost"])
		q, err = ParseLegacyAgentKeyQuote(cost)
		schema, input = data["params"], args["params"]
		if input == nil {
			input = map[string]any{}
		}
	} else {
		if data["provider"] != query["provider"] || data["endpoint"] != query["endpoint"] {
			return q, ErrPrice
		}
		price, _ := json.Marshal(data["price"])
		q, err = ParseLegacyMonidQuote(price)
		schemas, _ := data["schema"].(map[string]any)
		schema, input = data["inputSchema"], args["input"]
		if nested := schemas["input"]; nested != nil {
			if schema != nil && digest(schema) != digest(nested) {
				return q, ErrPrice
			}
			schema = nested
		}
		q.TargetProvider, q.TargetEndpoint = query["provider"].(string), query["endpoint"].(string)
	}
	if err != nil || q.Unit != "call" || schema == nil {
		return LegacyProviderQuote{}, ErrPrice
	}
	rawSchema, _ := json.Marshal(schema)
	rawInput, _ := json.Marshal(input)
	if err := validate(rawSchema, rawInput); err != nil {
		return LegacyProviderQuote{}, err
	}
	return q, nil
}
func legacyProviderPayload(exact map[string]any) (map[string]any, error) {
	if data, ok := exact["structuredContent"].(map[string]any); ok {
		return data, nil
	}
	content, ok := exact["content"].([]any)
	if !ok || len(content) != 1 {
		return nil, ErrPrice
	}
	item, ok := content[0].(map[string]any)
	if !ok || item["type"] != "text" {
		return nil, ErrPrice
	}
	text, ok := item["text"].(string)
	if !ok {
		return nil, ErrPrice
	}
	var data map[string]any
	if strictLegacyJSON([]byte(text), &data, false) != nil || data == nil {
		return nil, ErrPrice
	}
	return data, nil
}
func legacyProviderNumber(raw json.RawMessage) (string, error) {
	var s string
	if len(raw) > 0 && raw[0] == '"' {
		if json.Unmarshal(raw, &s) != nil {
			return "", ErrPrice
		}
	} else {
		s = string(raw)
	}
	if _, err := legacyProviderDecimal(s); err != nil {
		return "", ErrPrice
	}
	return s, nil
}
func legacyProviderMoney(raw json.RawMessage, currency string) (string, error) {
	if len(raw) == 0 {
		return "", ErrPrice
	}
	if raw[0] == '{' {
		var amount struct {
			Value    json.RawMessage `json:"value"`
			Currency string          `json:"currency"`
		}
		if strictLegacyJSON(raw, &amount, false) != nil || amount.Currency != "USD" || currency != "" && currency != amount.Currency {
			return "", ErrPrice
		}
		return legacyProviderNumber(amount.Value)
	}
	if currency != "USD" {
		return "", ErrPrice
	}
	return legacyProviderNumber(raw)
}
func ParseLegacyAgentKeyQuote(raw json.RawMessage) (LegacyProviderQuote, error) {
	q := LegacyProviderQuote{Provider: "agentkey", Unit: "call"}
	var cost struct {
		USD json.RawMessage `json:"usd_per_call"`
	}
	if strictLegacyJSON(raw, &cost, false) != nil {
		return q, ErrPrice
	}
	var err error
	q.AmountUSD, err = legacyProviderNumber(cost.USD)
	return q, err
}
func ParseLegacyMonidQuote(raw json.RawMessage) (LegacyProviderQuote, error) {
	q := LegacyProviderQuote{Provider: "monid"}
	var price struct {
		Type     string          `json:"type"`
		Amount   json.RawMessage `json:"amount"`
		Currency string          `json:"currency"`
		FlatFee  json.RawMessage `json:"flatFee"`
	}
	if strictLegacyJSON(raw, &price, false) != nil {
		return q, ErrPrice
	}
	switch price.Type {
	case "PER_CALL":
		q.Unit = "call"
	case "PER_RESULT":
		q.Unit = "result"
	default:
		return q, ErrPrice
	}
	var err error
	q.AmountUSD, err = legacyProviderMoney(price.Amount, price.Currency)
	if err != nil {
		return q, err
	}
	if len(price.FlatFee) > 0 && string(price.FlatFee) != "null" {
		q.FlatFeeUSD, err = legacyProviderMoney(price.FlatFee, "USD")
	}
	return q, err
}
