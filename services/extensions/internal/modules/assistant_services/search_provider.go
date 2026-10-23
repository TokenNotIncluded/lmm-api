// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later
package assistant_services

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"net/http/httptrace"
	"net/url"
	"strings"
	"sync/atomic"
	"time"
	"unicode/utf8"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/toolmarket"
)

// SearchConfig is an immutable, host-reviewed provider selection. Queries never
// supply endpoints, credentials or provider names. Secrets are injected by host.
type SearchConfig struct {
	Provider  string
	Endpoint  string
	APIKey    string
	MCPTool   string
	LocalHTTP bool
	Timeout   time.Duration
}

type SearchProvider interface {
	Search(context.Context, string) (json.RawMessage, error)
}

// SearchError distinguishes requests that may have reached the provider. The
// durable caller must not dispatch again after DispatchUnknown returns true.
type SearchError struct{ Unknown bool }

func (e *SearchError) Error() string {
	if e.Unknown {
		return "search provider outcome is unknown"
	}
	return "search request was not dispatched"
}
func (e *SearchError) DispatchUnknown() bool { return e.Unknown }

type httpSearchProvider struct {
	config SearchConfig
	client *http.Client
}

func NewSearchProvider(config SearchConfig) (SearchProvider, error) {
	config.Provider = strings.TrimSpace(config.Provider)
	config.Endpoint = strings.TrimSpace(config.Endpoint)
	config.MCPTool = strings.TrimSpace(config.MCPTool)
	if len(config.MCPTool) > 128 || strings.ContainsAny(config.MCPTool, "\r\n\x00") {
		return nil, errors.New("invalid MCP tool selection")
	}
	config.APIKey = strings.TrimSpace(config.APIKey)
	if config.Provider == "" {
		config.Provider = "generic_http"
	}
	switch config.Provider {
	case "none":
		return &httpSearchProvider{config: config}, nil
	case "exa":
		if config.Endpoint == "" {
			config.Endpoint = "https://api.exa.ai/search"
		}
	case "tavily":
		if config.Endpoint == "" {
			config.Endpoint = "https://api.tavily.com/search"
		}
	case "brave":
		if config.Endpoint == "" {
			config.Endpoint = "https://api.search.brave.com/res/v1/web/search"
		}
	case "generic_http", "mcp_streamable_http":
	default:
		return nil, errors.New("unsupported configured search provider")
	}
	if config.Provider != "generic_http" && config.Provider != "mcp_streamable_http" && config.APIKey == "" {
		return nil, errors.New("configured search provider requires a credential")
	}
	if len(config.APIKey) > 4096 || strings.ContainsAny(config.APIKey, "\r\n\x00") {
		return nil, errors.New("invalid configured search credential")
	}
	endpoint, err := url.Parse(config.Endpoint)
	if err != nil || endpoint.Host == "" || len(config.Endpoint) > 2048 || endpoint.User != nil || endpoint.Fragment != "" || endpoint.Opaque != "" || strings.ContainsAny(config.Endpoint, "\r\n\\") {
		return nil, errors.New("invalid configured search endpoint")
	}
	// The shared transport repeats endpoint policy at every request; construction
	// also rejects a bad host selection before any durable dispatch is attempted.
	if err := toolmarket.ValidateExternalHTTPURL(config.Endpoint, config.LocalHTTP); err != nil {
		return nil, errors.New("search endpoint is denied")
	}
	client, err := toolmarket.NewExternalHTTPClient(config.LocalHTTP)
	if err != nil {
		return nil, err
	}
	if config.Timeout == 0 {
		config.Timeout = 10 * time.Second
	}
	if config.Timeout < time.Millisecond || config.Timeout > 30*time.Second {
		return nil, errors.New("search timeout is out of bounds")
	}
	client.Timeout = config.Timeout
	return &httpSearchProvider{config: config, client: client}, nil
}

func (p *httpSearchProvider) request(ctx context.Context, query string) (*http.Request, error) {
	endpoint, err := url.Parse(p.config.Endpoint)
	if err != nil {
		return nil, err
	}
	method := http.MethodGet
	var body io.Reader
	switch p.config.Provider {
	case "exa":
		method = http.MethodPost
		b, _ := json.Marshal(map[string]any{"query": query, "contents": map[string]any{"highlights": true}})
		body = bytes.NewReader(b)
	case "tavily":
		method = http.MethodPost
		b, _ := json.Marshal(map[string]any{"query": query})
		body = bytes.NewReader(b)
	case "brave", "generic_http":
		values := endpoint.Query()
		values.Set("q", query)
		endpoint.RawQuery = values.Encode()
	default:
		return nil, errors.New("search is disabled")
	}
	request, err := http.NewRequestWithContext(ctx, method, endpoint.String(), body)
	if err != nil {
		return nil, err
	}
	request.Header.Set("Accept", "application/json")
	if body != nil {
		request.Header.Set("Content-Type", "application/json")
	}
	switch p.config.Provider {
	case "exa":
		request.Header.Set("x-api-key", p.config.APIKey)
	case "tavily":
		request.Header.Set("Authorization", "Bearer "+p.config.APIKey)
	case "brave":
		request.Header.Set("X-Subscription-Token", p.config.APIKey)
	case "generic_http":
		if p.config.APIKey != "" {
			request.Header.Set("Authorization", "Bearer "+p.config.APIKey)
			request.Header.Set("X-API-Key", p.config.APIKey)
		}
	}
	return request, nil
}

func searchEnvelope(configured bool, query string, status int, results any, ok bool) json.RawMessage {
	raw, err := json.Marshal(map[string]any{"ok": ok, "configured": configured, "query": query, "status": status, "results": results, "source_is_untrusted": true})
	if err != nil || len(raw) > 64*1024 {
		return nil
	}
	return raw
}

func redactSearchValue(value any, secret string) any {
	if secret == "" {
		return value
	}
	switch v := value.(type) {
	case string:
		return strings.ReplaceAll(v, secret, "[REDACTED]")
	case float64:
		encoded, _ := json.Marshal(v)
		if string(encoded) == secret {
			return "[REDACTED]"
		}
	case bool:
		encoded, _ := json.Marshal(v)
		if string(encoded) == secret {
			return "[REDACTED]"
		}
	case []any:
		for i := range v {
			v[i] = redactSearchValue(v[i], secret)
		}
		return v
	case map[string]any:
		clean := make(map[string]any, len(v))
		for key, item := range v {
			clean[strings.ReplaceAll(key, secret, "[REDACTED]")] = redactSearchValue(item, secret)
		}
		return clean
	}
	return value
}

func (p *httpSearchProvider) Search(ctx context.Context, query string) (json.RawMessage, error) {
	query = strings.TrimSpace(query)
	configured := p.config.Provider != "none"
	displayQuery := query
	if p.config.APIKey != "" {
		displayQuery = strings.ReplaceAll(query, p.config.APIKey, "[REDACTED]")
	}
	failure := func(unknown bool, status int) (json.RawMessage, error) {
		return searchEnvelope(configured, displayQuery, status, nil, false), &SearchError{Unknown: unknown}
	}
	if !utf8.ValidString(query) || utf8.RuneCountInString(query) < 2 || utf8.RuneCountInString(query) > 2000 || strings.ContainsRune(query, 0) || !configured {
		return failure(false, 0)
	}
	if ctx == nil {
		ctx = context.Background()
	}
	if p.config.Provider == "mcp_streamable_http" {
		return p.searchMCP(ctx, query, displayQuery)
	}
	var connected atomic.Bool
	ctx = httptrace.WithClientTrace(ctx, &httptrace.ClientTrace{GotConn: func(httptrace.GotConnInfo) { connected.Store(true) }})
	request, err := p.request(ctx, query)
	if err != nil {
		return failure(false, 0)
	}
	response, err := p.client.Do(request)
	if err != nil {
		return failure(connected.Load(), 0)
	}
	defer response.Body.Close()
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		return failure(true, response.StatusCode)
	}
	body, err := io.ReadAll(io.LimitReader(response.Body, 64*1024+1))
	if err != nil || len(body) > 64*1024 {
		return failure(true, response.StatusCode)
	}
	var results any
	if json.Unmarshal(body, &results) != nil {
		results = strings.TrimSpace(string(body))
	}
	results = redactSearchValue(results, p.config.APIKey)
	result := searchEnvelope(true, displayQuery, response.StatusCode, results, true)
	if result == nil {
		return failure(true, response.StatusCode)
	}
	return result, nil
}
