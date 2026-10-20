// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later
// Selection/result semantics ported from fixed main 3948a0a1 assistant_search.go.
package assistant_services

import (
	"context"
	"encoding/json"
	"errors"
	"github.com/modelcontextprotocol/go-sdk/mcp"
	"io"
	"net/http"
	"net/http/httptrace"
	"sort"
	"strings"
	"sync/atomic"
)

// The official SDK implements MCP. Its retry/resume mechanisms are explicitly
// disabled; the outer durable operation marker controls recovery.
func (p *httpSearchProvider) searchMCP(ctx context.Context, query, displayQuery string) (json.RawMessage, error) {
	ctx, cancel := context.WithTimeout(ctx, p.config.Timeout)
	defer cancel()
	var dispatched, failed atomic.Bool
	ctx = httptrace.WithClientTrace(ctx, &httptrace.ClientTrace{GotConn: func(httptrace.GotConnInfo) { dispatched.Store(true) }})
	client := *p.client
	client.Transport = mcpSearchTransport{base: p.client.Transport, endpoint: p.config.Endpoint, token: p.config.APIKey, failed: &failed}
	sdk := mcp.NewClient(&mcp.Implementation{Name: "api.lmm.best-assistant-search", Version: "1.0.0"}, &mcp.ClientOptions{MultiRoundTrip: &mcp.MultiRoundTripOptions{Disabled: true}})
	session, err := sdk.Connect(ctx, &mcp.StreamableClientTransport{Endpoint: p.config.Endpoint, HTTPClient: &client, MaxRetries: -1, DisableStandaloneSSE: true}, nil)
	failure := func() (json.RawMessage, error) { return nil, &SearchError{Unknown: dispatched.Load()} }
	if err != nil {
		return failure()
	}
	defer session.Close()
	tools, err := session.ListTools(ctx, nil)
	if err != nil || tools == nil || len(tools.Tools) > 256 {
		return failure()
	}
	tool, err := selectAssistantMCPTool(tools.Tools, p.config.MCPTool)
	if err != nil {
		return failure()
	}
	value, err := session.CallTool(ctx, &mcp.CallToolParams{Name: tool.Name, Arguments: map[string]any{assistantMCPQueryArgument(tool): query}})
	if err != nil {
		return failure()
	}
	data, err := assistantMCPResultData(value)
	if err != nil {
		return failure()
	}
	out := searchEnvelope(true, displayQuery, 200, redactSearchValue(data, p.config.APIKey), true)
	if out == nil {
		return failure()
	}
	return out, nil
}

type mcpSearchTransport struct {
	base            http.RoundTripper
	endpoint, token string
	failed          *atomic.Bool
}

func (t mcpSearchTransport) RoundTrip(request *http.Request) (*http.Response, error) {
	if t.failed.Load() {
		return nil, errors.New("MCP transport already failed")
	}
	if request.URL.String() != t.endpoint {
		return nil, errors.New("MCP endpoint changed")
	}
	clone := request.Clone(request.Context())
	clone.Header = request.Header.Clone()
	if t.token != "" {
		clone.Header.Set("Authorization", "Bearer "+t.token)
	}
	response, err := t.base.RoundTrip(clone)
	if err != nil {
		t.failed.Store(true)
		return nil, err
	}
	if response.StatusCode == 429 || response.StatusCode >= 500 {
		t.failed.Store(true)
	}
	if response.StatusCode >= 300 && response.StatusCode < 400 {
		t.failed.Store(true)
		response.Body.Close()
		return nil, errors.New("MCP redirect denied")
	}
	response.Body = &mcpSearchBody{ReadCloser: response.Body, remaining: 65536, failed: t.failed}
	return response, nil
}

type mcpSearchBody struct {
	io.ReadCloser
	remaining int
	failed    *atomic.Bool
}

func (b *mcpSearchBody) Read(p []byte) (int, error) {
	if b.remaining == 0 {
		var probe [1]byte
		n, err := b.ReadCloser.Read(probe[:])
		if n > 0 {
			b.failed.Store(true)
			return 0, errors.New("MCP response limit exceeded")
		}
		return 0, err
	}
	if len(p) > b.remaining {
		p = p[:b.remaining]
	}
	n, err := b.ReadCloser.Read(p)
	b.remaining -= n
	return n, err
}
func boundedMCPMarshal(value any, limit int) ([]byte, error) {
	raw, err := json.Marshal(value)
	if err != nil || len(raw) > limit {
		return nil, errors.New("MCP result limit exceeded")
	}
	return raw, nil
}
func selectAssistantMCPTool(tools []*mcp.Tool, configuredName string) (*mcp.Tool, error) {
	configuredName = strings.TrimSpace(configuredName)
	if configuredName != "" {
		for _, tool := range tools {
			if tool != nil && tool.Name == configuredName {
				return tool, nil
			}
		}
		return nil, errors.New("configured MCP search tool was not found")
	}

	type candidate struct {
		tool  *mcp.Tool
		score int
	}
	candidates := make([]candidate, 0, len(tools))
	for _, tool := range tools {
		if tool == nil {
			continue
		}
		name := strings.ToLower(strings.TrimSpace(tool.Name))
		score := -1
		switch {
		case name == "web_search":
			score = 0
		case name == "search":
			score = 1
		case strings.Contains(name, "web_search"):
			score = 2
		case strings.Contains(name, "search"):
			score = 3
		case strings.Contains(strings.ToLower(tool.Title), "search") || strings.Contains(strings.ToLower(tool.Description), "search"):
			score = 4
		}
		if score >= 0 {
			candidates = append(candidates, candidate{tool: tool, score: score})
		}
	}
	if len(candidates) == 0 {
		return nil, errors.New("MCP server does not expose a search tool")
	}
	sort.SliceStable(candidates, func(i, j int) bool {
		if candidates[i].score != candidates[j].score {
			return candidates[i].score < candidates[j].score
		}
		return candidates[i].tool.Name < candidates[j].tool.Name
	})
	return candidates[0].tool, nil
}

func assistantMCPQueryArgument(tool *mcp.Tool) string {
	if tool == nil {
		return "query"
	}
	schema, ok := tool.InputSchema.(map[string]any)
	if !ok {
		return "query"
	}
	properties, ok := schema["properties"].(map[string]any)
	if !ok {
		return "query"
	}
	for _, name := range []string{"query", "q", "search_query", "keyword"} {
		if _, exists := properties[name]; exists {
			return name
		}
	}
	return "query"
}

func assistantMCPResultData(result *mcp.CallToolResult) (any, error) {
	if result == nil {
		return nil, errors.New("MCP search returned an empty result")
	}
	if result.IsError {
		return nil, errors.New("MCP search tool returned an error")
	}
	if result.StructuredContent != nil {
		// The MCP SDK has already decoded the structured value, so returning it
		// directly would let a provider-sized object survive until the agent
		// marshals the tool result. Re-encode through the same hard response
		// budget used by the regular HTTP search adapter before handing data to
		// the agent. This keeps both the retained value and the eventual tool
		// context bounded, including a structured array with a large number of
		// entries.
		bounded, err := boundedMCPMarshal(result.StructuredContent, 65536)
		if err != nil {
			return nil, errors.New("MCP search result is too large")
		}
		var decoded any
		if err := json.Unmarshal(bounded, &decoded); err != nil {
			return nil, errors.New("MCP search result is invalid")
		}
		return decoded, nil
	}
	texts := make([]string, 0, len(result.Content))
	for _, content := range result.Content {
		if text, ok := content.(*mcp.TextContent); ok {
			if len(text.Text) > 65536 {
				return nil, errors.New("MCP search result is too large")
			}
			texts = append(texts, text.Text)
		}
	}
	if len(texts) == 0 {
		return map[string]any{"content": []any{}}, nil
	}
	if len(texts) == 1 {
		var decoded any
		if json.Unmarshal([]byte(texts[0]), &decoded) == nil {
			return decoded, nil
		}
		return texts[0], nil
	}
	bounded, err := boundedMCPMarshal(texts, 65536)
	if err != nil {
		return nil, errors.New("MCP search result is too large")
	}
	var decoded []string
	if err := json.Unmarshal(bounded, &decoded); err != nil {
		return nil, errors.New("MCP search result is invalid")
	}
	return decoded, nil
}
