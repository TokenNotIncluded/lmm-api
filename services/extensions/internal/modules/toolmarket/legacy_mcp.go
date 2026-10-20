// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package toolmarket

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"io"
	"net"
	"net/http"
	"net/netip"
	"net/url"
	"sort"
	"strconv"
	"strings"

	"github.com/modelcontextprotocol/go-sdk/mcp"
)

type LegacyMarketMCPOptions struct {
	LocalHTTP bool
	// Both values come from trusted host configuration, never request Host.
	Issuer              string
	Resource            string
	ResourceMetadataURL string
}

func legacyMCPOptionsValid(opts LegacyMarketMCPOptions) bool {
	resource, e := url.Parse(opts.Resource)
	if e != nil || resource.Path != "/mcp/market" {
		return false
	}
	var same netip.Addr
	for _, raw := range []string{opts.Issuer, opts.Resource, opts.ResourceMetadataURL} {
		if !legacyAbsoluteResource(raw) {
			return false
		}
		u, _ := url.Parse(raw)
		if !opts.LocalHTTP {
			if u.Scheme != "https" {
				return false
			}
			continue
		}
		if u.Scheme != "http" {
			return false
		}
		ip, e := netip.ParseAddr(u.Hostname())
		if e != nil || !ip.IsLoopback() || ip.Zone() != "" {
			return false
		}
		ip = ip.Unmap()
		port, e := strconv.Atoi(u.Port())
		if e != nil || port < 1024 || port > 65535 {
			return false
		}
		if same.IsValid() && ip != same {
			return false
		}
		same = ip
	}
	return true
}
func legacyMCPChallenge(opts LegacyMarketMCPOptions, scopes []string, code string) string {
	parts := []string{"Bearer resource_metadata=" + strconv.Quote(opts.ResourceMetadataURL)}
	if len(scopes) > 0 {
		parts = append(parts, "scope="+strconv.Quote(strings.Join(scopes, " ")))
	}
	if code != "" {
		parts = append(parts, "error="+strconv.Quote(code))
	}
	return strings.Join(parts, ", ")
}
func legacyMCPRequired(raw []byte) (invoke, manage bool) {
	var calls []json.RawMessage
	if bytes.HasPrefix(bytes.TrimSpace(raw), []byte("[")) {
		_ = json.Unmarshal(raw, &calls)
	} else {
		calls = []json.RawMessage{raw}
	}
	for _, r := range calls {
		var msg struct {
			Method string `json:"method"`
			Params struct {
				Name      string `json:"name"`
				Arguments struct {
					Action string `json:"action"`
				} `json:"arguments"`
			} `json:"params"`
		}
		if json.Unmarshal(r, &msg) != nil || msg.Method != "tools/call" {
			continue
		}
		switch {
		case strings.HasPrefix(msg.Params.Name, "market_tool_"):
			invoke = true
		case msg.Params.Name == "lmm_market_load":
			manage = true
		case msg.Params.Name == "metamcp":
			switch msg.Params.Arguments.Action {
			case "invoke":
				invoke = true
			case "authorize":
				invoke = true
				manage = true
			case "load", "unload", "set_tool_budget", "set_client_budget":
				manage = true
			}
		}
	}
	return
}
func (m *Module) LegacyMarketMCPHandler(opts LegacyMarketMCPOptions) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Cache-Control", "no-store")
		w.Header().Set("X-Content-Type-Options", "nosniff")
		w.Header().Set("Access-Control-Allow-Origin", "*")
		w.Header().Set("Access-Control-Allow-Methods", "POST, GET, DELETE, OPTIONS")
		w.Header().Set("Access-Control-Allow-Headers", "Authorization, Content-Type, Accept, Mcp-Protocol-Version, Mcp-Session-Id")
		w.Header().Set("Access-Control-Expose-Headers", "WWW-Authenticate, Mcp-Session-Id")
		if r.Method == http.MethodOptions {
			w.WriteHeader(http.StatusNoContent)
			return
		}
		if r.URL.Path != "/mcp/market" && r.URL.Path != "/mcp/market/" {
			http.NotFound(w, r)
			return
		}
		if !legacyMCPOptionsValid(opts) {
			http.Error(w, "MCP configuration unavailable", 503)
			return
		}
		if !m.legacyMCPAcknowledged() {
			http.Error(w, "MCP Core capability acknowledgement unavailable", 503)
			return
		}
		values := r.Header.Values("Authorization")
		raw := ""
		if len(values) == 1 && len(values[0]) > 7 && strings.EqualFold(values[0][:7], "Bearer ") {
			raw = values[0][7:]
		}
		p, e := m.LegacyAuthenticateMarket(r.Context(), raw, []string{"market:discover"})
		if e != nil {
			status, code := 401, "invalid_token"
			if raw == "" {
				code = ""
			}
			if errors.Is(e, ErrLegacyMarketScope) {
				status, code = 403, "insufficient_scope"
			}
			if errors.Is(e, ErrUnavailable) {
				http.Error(w, "MCP authority unavailable", 503)
				return
			}
			w.Header().Set("WWW-Authenticate", legacyMCPChallenge(opts, []string{"market:discover"}, code))
			http.Error(w, "MCP authorization required", status)
			return
		}
		if p.CredentialKind == "oauth" && (p.Issuer != opts.Issuer || p.Resource != opts.Resource) {
			w.Header().Set("WWW-Authenticate", legacyMCPChallenge(opts, nil, "invalid_token"))
			http.Error(w, "MCP authorization required", 401)
			return
		}
		q, qe := url.ParseQuery(r.URL.RawQuery)
		mode := q.Get("mode")
		alternate := false
		for _, header := range []string{"X-Account-ID", "X-User-ID", "X-Api-Key", "Api-Key", "X-Goog-Api-Key", "Mj-Api-Secret", "Sec-Websocket-Protocol", "New-Api-User", "Content-Encoding", "X-LMM-Group", "X-OAuth-Group", "X-Lmm-User-Credential", "Cookie"} {
			if len(r.Header.Values(header)) > 0 {
				alternate = true
			}
		}
		if qe != nil || len(q) > 1 || len(q) == 1 && len(q["mode"]) != 1 || mode != "" && mode != "full" && mode != "compact" || alternate {
			http.Error(w, "MCP accepts one bearer and optional full or compact mode", 400)
			return
		}
		r.Body = http.MaxBytesReader(w, r.Body, 256<<10)
		if r.Method == http.MethodPost {
			body, readErr := io.ReadAll(r.Body)
			if readErr != nil {
				http.Error(w, "MCP request too large", 413)
				return
			}
			r.Body = io.NopCloser(bytes.NewReader(body))
			invoke, manage := legacyMCPRequired(body)
			if invoke && !p.CanInvoke || manage && !p.CanManage {
				scopes := []string{"market:discover"}
				if invoke || p.CanInvoke {
					scopes = append(scopes, "market:invoke")
				}
				if manage || p.CanManage {
					scopes = append(scopes, "market:manage")
				}
				w.Header().Set("WWW-Authenticate", legacyMCPChallenge(opts, scopes, "insufficient_scope"))
				http.Error(w, "MCP connection needs additional permissions", 403)
				return
			}
		}
		ctx := r.Context()
		if host, _, err := net.SplitHostPort(r.RemoteAddr); err == nil {
			ctx = WithBuiltinPeerIP(ctx, host)
		}
		r = r.WithContext(ctx)
		server, e := m.legacyMCPServer(ctx, raw, p, mode == "compact")
		if e != nil {
			http.Error(w, "Tool set temporarily unavailable", 503)
			return
		}
		handler := mcp.NewStreamableHTTPHandler(func(*http.Request) *mcp.Server { return server }, &mcp.StreamableHTTPOptions{Stateless: true, JSONResponse: true, DisableLocalhostProtection: true, PropagateRequestCancellation: true})
		handler.ServeHTTP(w, r)
	})
}

func (m *Module) legacyMCPServer(ctx context.Context, raw string, p LegacyMarketPrincipal, compact bool) (*mcp.Server, error) {
	server := mcp.NewServer(&mcp.Implementation{Name: "lmm-tool-market", Version: "2"}, nil)
	server.AddTool(LegacyMetaToolDescription(), func(ctx context.Context, req *mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		in, e := legacyDecodeMCPInput(req.Params.Arguments, m.now().Unix())
		if e != nil {
			return legacyMCPOutput(nil, e)
		}
		if in.Action == "invoke" {
			if !p.CanInvoke {
				return legacyMCPOutput(nil, ErrLegacyMarketScope)
			}
			return m.legacyMCPInvoke(ctx, raw, p, in, req)
		}
		if req.Params.RequestState != "" || len(req.Params.InputResponses) > 0 {
			return legacyMCPOutput(nil, ErrInvalid)
		}
		value, e := m.legacyMCPMeta(ctx, raw, p, in)
		if compact && e == nil {
			if view, ok := value.(map[string]any); ok {
				if _, exists := view["refresh_tools_list"]; exists {
					view["refresh_tools_list"] = false
				}
			}
		}
		return legacyMCPOutput(value, e)
	})
	if compact {
		return server, nil
	}
	server.AddTool(&mcp.Tool{Name: "lmm_market_search", Description: "Search service summaries. Use details to read exact tool schemas and prices.", InputSchema: legacyMCPSchema(map[string]any{"query": map[string]any{"type": "string", "maxLength": 120}})}, func(ctx context.Context, req *mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		var in struct {
			Query string `json:"query"`
		}
		if e := decodeStrict(req.Params.Arguments, &in); e != nil {
			return legacyMCPOutput(nil, e)
		}
		value, e := m.legacyMCPMeta(ctx, raw, p, legacyMCPInput{Action: "search", Query: in.Query, Limit: 5})
		return legacyMCPOutput(value, e)
	})
	for _, entry := range []struct{ name, action, key string }{{"lmm_market_details", "details", "service_id"}, {"lmm_market_call_status", "call_status", "call_id"}} {
		entry := entry
		server.AddTool(&mcp.Tool{Name: entry.name, InputSchema: legacyMCPSchema(map[string]any{entry.key: map[string]any{"type": "string", "minLength": 1, "maxLength": 128}}, entry.key)}, func(ctx context.Context, req *mcp.CallToolRequest) (*mcp.CallToolResult, error) {
			var obj map[string]json.RawMessage
			if decodeStrict(req.Params.Arguments, &obj) != nil || len(obj) != 1 {
				return legacyMCPOutput(nil, ErrInvalid)
			}
			obj["action"], _ = json.Marshal(entry.action)
			b, _ := json.Marshal(obj)
			in, e := legacyDecodeMCPInput(b, m.now().Unix())
			if e != nil {
				return legacyMCPOutput(nil, e)
			}
			value, e := m.legacyMCPMeta(ctx, raw, p, in)
			return legacyMCPOutput(value, e)
		})
	}
	if p.CanManage {
		server.AddTool(&mcp.Tool{Name: "lmm_market_load", InputSchema: legacyMCPSchema(map[string]any{"tool_id": map[string]any{"type": "string"}, "version_id": map[string]any{"type": "string"}, "loaded": map[string]any{"type": "boolean"}}, "tool_id", "version_id", "loaded")}, func(ctx context.Context, req *mcp.CallToolRequest) (*mcp.CallToolResult, error) {
			v, e := payload[struct {
				ToolID    string `json:"tool_id"`
				VersionID string `json:"version_id"`
				Loaded    bool   `json:"loaded"`
			}](req.Params.Arguments, "tool_id", "version_id", "loaded")
			if e != nil {
				return legacyMCPOutput(nil, e)
			}
			action := "unload"
			if v.Loaded {
				action = "load"
			}
			value, e := m.legacyMCPMeta(ctx, raw, p, legacyMCPInput{Action: action, ToolID: v.ToolID, VersionID: v.VersionID})
			return legacyMCPOutput(value, e)
		})
	}
	if !p.CanInvoke {
		return server, nil
	}
	grants, e := m.legacyMCPGrants(ctx, raw, p)
	if e != nil {
		return nil, e
	}
	user, _ := strconv.ParseInt(p.UserID, 10, 64)
	var tools []WebTool
	e = m.store.view(func(d database) error {
		for _, i := range webState(&d).Installations {
			if i.UserID != user || i.ClientID != p.ClientID {
				continue
			}
			if _, ok := legacyMCPGrant(grants, i.ToolID, i.VersionID, m.now().Unix()); !ok {
				continue
			}
			snapshot, exists := webState(&d).Versions[i.VersionID]
			if !exists || snapshot.Version.ExecutionType != "remote" && snapshot.Version.ExecutionType != "builtin" {
				continue
			}
			tool, e := legacyLiveTool(&d, user, i.ToolID, i.VersionID)
			if e == nil {
				tools = append(tools, tool)
			}
		}
		return nil
	})
	if e != nil {
		return nil, e
	}
	sort.Slice(tools, func(i, j int) bool { return tools[i].ToolID < tools[j].ToolID })
	seenNames := map[string]bool{}
	for _, t := range tools {
		t := t
		name := "market_tool_" + strings.ReplaceAll(t.ToolID, "-", "")
		if seenNames[name] {
			return nil, ErrConflict
		}
		seenNames[name] = true
		schema, e := legacyMCPWrappedSchema(t.InputSchema)
		if e != nil {
			return nil, e
		}
		definition := &mcp.Tool{Name: name, Title: t.Name, Description: t.Description, InputSchema: schema, Meta: mcp.Meta{"lmm/pricing": legacyMCPPricing(t)}}
		if len(t.Annotations) > 0 {
			var hints mcp.ToolAnnotations
			if json.Unmarshal(t.Annotations, &hints) != nil {
				return nil, ErrInvalid
			}
			definition.Annotations = &hints
		}
		if t.OutputSchema != "" {
			definition.OutputSchema = json.RawMessage(t.OutputSchema)
		}
		server.AddTool(definition, func(ctx context.Context, req *mcp.CallToolRequest) (*mcp.CallToolResult, error) {
			in, e := payload[struct {
				RequestID string          `json:"request_id"`
				Arguments json.RawMessage `json:"arguments"`
			}](req.Params.Arguments, "request_id", "arguments")
			if e != nil {
				return legacyMCPOutput(nil, e)
			}
			return m.legacyMCPInvoke(ctx, raw, p, legacyMCPInput{Action: "invoke", ToolID: t.ToolID, VersionID: t.VersionID, RequestID: in.RequestID, Arguments: in.Arguments}, req)
		})
	}
	return server, nil
}
func legacyMCPWrappedSchema(raw string) (map[string]any, error) {
	var original any
	if decodeStrict([]byte(raw), &original) != nil {
		return nil, ErrInvalid
	}
	var rewrite func(any)
	rewrite = func(v any) {
		switch x := v.(type) {
		case map[string]any:
			for k, c := range x {
				if k == "$ref" {
					if s, ok := c.(string); ok && (s == "#" || strings.HasPrefix(s, "#/")) {
						x[k] = "#/$defs/arguments" + strings.TrimPrefix(s, "#")
					}
				} else {
					rewrite(c)
				}
			}
		case []any:
			for _, c := range x {
				rewrite(c)
			}
		}
	}
	rewrite(original)
	out := legacyMCPSchema(map[string]any{"request_id": map[string]any{"type": "string", "minLength": 1, "maxLength": 128}, "arguments": map[string]any{"$ref": "#/$defs/arguments"}}, "request_id", "arguments")
	out["$defs"] = map[string]any{"arguments": original}
	return out, nil
}

func legacyMCPPricing(t WebTool) map[string]any {
	pricing := map[string]any{"price_quota": t.PriceQuota, "billing_mode": t.BillingMode, "input_token_price_quota": t.InputTokenPriceQuota, "max_input_tokens": t.MaxInputTokens, "billing_rules": t.BillingRules}
	if t.ProviderPricing != nil {
		pricing["billing_mode"] = "provider_quote"
		pricing["provider_pricing"] = t.ProviderPricing
		pricing["maximum_price_quota"] = t.PriceQuota
		pricing["price_quota"] = nil
		pricing["usage_policy"] = "fresh_upstream_usd_quote"
	}
	if t.BillingMode != "" {
		pricing["usage_policy"] = "tool_reported"
	}
	return pricing
}
