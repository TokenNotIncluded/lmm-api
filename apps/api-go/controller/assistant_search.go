package controller

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"io"
	"net"
	"net/http"
	"net/url"
	"sort"
	"strings"
	"time"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/LIghtJUNction/api.lmm.best/setting"
	"github.com/modelcontextprotocol/go-sdk/mcp"
)

const (
	assistantSearchDefaultTimeout   = 10 * time.Second
	assistantSearchMaxResponseBytes = 64 * 1024
	assistantSearchMaxQueryRunes    = 2000
	assistantSearchMaxRedirects     = 3

	assistantSearchExaURL    = "https://api.exa.ai/search"
	assistantSearchTavilyURL = "https://api.tavily.com/search"
	assistantSearchBraveURL  = "https://api.search.brave.com/res/v1/web/search"
)

// AssistantSearchResponse is deliberately provider-neutral so the assistant
// agent can keep its existing tool response shape while delegating all remote
// search behavior to this file.
type AssistantSearchResponse struct {
	Configured bool
	Query      string
	Status     int
	Results    any
}

// ExecuteAssistantSearch executes the administrator-selected search provider.
// It does not expose the configured API key and returns only bounded provider
// data. The assistant agent owns the user-facing error mapping.
func ExecuteAssistantSearch(ctx context.Context, query string) (AssistantSearchResponse, error) {
	return executeAssistantSearch(ctx, setting.GetAssistantSettings(), query, nil)
}

func executeAssistantSearch(ctx context.Context, settings setting.AssistantSettings, query string, client *http.Client) (AssistantSearchResponse, error) {
	query = strings.TrimSpace(query)
	response := AssistantSearchResponse{Query: query}
	if len([]rune(query)) < 2 {
		return response, errors.New("search query is required")
	}
	if len([]rune(query)) > assistantSearchMaxQueryRunes {
		return response, errors.New("search query is too long")
	}
	if ctx == nil {
		ctx = context.Background()
	}

	provider := settings.SearchProvider
	// A missing provider option means this is an older installation. Treat it
	// as the legacy generic HTTP adapter so an existing SearchURL continues to
	// work. An explicit "none" still disables search.
	if provider == "" {
		provider = setting.DefaultAssistantSearchProvider
	}
	if !setting.IsAssistantSearchProvider(provider) {
		return response, errors.New("configured search provider is invalid")
	}
	response.Configured = provider != setting.AssistantSearchProviderNone
	if provider == setting.AssistantSearchProviderNone {
		response.Configured = false
		return response, errors.New("web search is not configured by the administrator")
	}

	endpoint, err := assistantSearchEndpoint(provider, settings.SearchURL)
	if err != nil {
		return response, err
	}

	if provider == setting.AssistantSearchProviderMCPStreamableHTTP {
		if client == nil {
			client = newAssistantSearchHTTPClient()
		}
		results, err := executeAssistantMCP(ctx, endpoint, settings.SearchAPIKey, settings.SearchMCPTool, query, client)
		if err != nil {
			return response, err
		}
		response.Results = results
		return response, nil
	}

	request, err := buildAssistantSearchRequest(ctx, settings, query)
	if err != nil {
		return response, err
	}
	if client == nil {
		client = newAssistantSearchHTTPClient()
	}
	providerResponse, err := client.Do(request)
	if err != nil {
		return response, errors.New("search provider request failed")
	}
	defer providerResponse.Body.Close()
	response.Status = providerResponse.StatusCode

	body, err := readAssistantSearchResponse(providerResponse.Body)
	if err != nil {
		return response, errors.New("search provider response could not be read")
	}
	if providerResponse.StatusCode < http.StatusOK || providerResponse.StatusCode >= http.StatusMultipleChoices {
		return response, errors.New("search provider returned an error")
	}

	var results any
	if err := json.Unmarshal(body, &results); err != nil {
		results = strings.TrimSpace(string(body))
	}
	response.Results = results
	return response, nil
}

func assistantSearchEndpoint(provider setting.AssistantSearchProvider, configuredURL string) (string, error) {
	configuredURL = strings.TrimSpace(configuredURL)
	if provider == setting.AssistantSearchProviderNone {
		return "", nil
	}
	if configuredURL != "" {
		if err := setting.ValidateAssistantSearchURL(configuredURL); err != nil {
			return "", errors.New("configured search URL is invalid")
		}
		return configuredURL, nil
	}

	switch provider {
	case setting.AssistantSearchProviderExa:
		return assistantSearchExaURL, nil
	case setting.AssistantSearchProviderTavily:
		return assistantSearchTavilyURL, nil
	case setting.AssistantSearchProviderBrave:
		return assistantSearchBraveURL, nil
	case setting.AssistantSearchProviderGenericHTTP:
		return "", errors.New("generic HTTP search requires a SearchURL")
	case setting.AssistantSearchProviderMCPStreamableHTTP:
		return "", errors.New("MCP search requires a SearchURL")
	default:
		return "", errors.New("configured search provider is invalid")
	}
}

func buildAssistantSearchRequest(ctx context.Context, settings setting.AssistantSettings, query string) (*http.Request, error) {
	provider := settings.SearchProvider
	if provider == "" {
		provider = setting.DefaultAssistantSearchProvider
	}
	endpoint, err := assistantSearchEndpoint(provider, settings.SearchURL)
	if err != nil {
		return nil, err
	}
	if provider == setting.AssistantSearchProviderNone {
		return nil, errors.New("web search is disabled")
	}
	if provider == setting.AssistantSearchProviderMCPStreamableHTTP {
		return nil, errors.New("MCP search does not use a regular HTTP search request")
	}
	if err := setting.ValidateAssistantSearchURL(endpoint); err != nil {
		return nil, errors.New("configured search URL is invalid")
	}
	query = strings.TrimSpace(query)
	if len([]rune(query)) < 2 || len([]rune(query)) > assistantSearchMaxQueryRunes {
		return nil, errors.New("search query is invalid")
	}
	apiKey := strings.TrimSpace(settings.SearchAPIKey)

	method := http.MethodGet
	var body io.Reader
	switch provider {
	case setting.AssistantSearchProviderExa:
		if apiKey == "" {
			return nil, errors.New("Exa search API key is not configured")
		}
		method = http.MethodPost
		payload := map[string]any{
			"query":    query,
			"contents": map[string]any{"highlights": true},
		}
		bodyBytes, err := json.Marshal(payload)
		if err != nil {
			return nil, errors.New("Exa search request could not be encoded")
		}
		body = bytes.NewReader(bodyBytes)
	case setting.AssistantSearchProviderTavily:
		if apiKey == "" {
			return nil, errors.New("Tavily search API key is not configured")
		}
		method = http.MethodPost
		bodyBytes, err := json.Marshal(map[string]any{"query": query})
		if err != nil {
			return nil, errors.New("Tavily search request could not be encoded")
		}
		body = bytes.NewReader(bodyBytes)
	case setting.AssistantSearchProviderBrave:
		if apiKey == "" {
			return nil, errors.New("Brave search API key is not configured")
		}
		parsed, err := url.Parse(endpoint)
		if err != nil {
			return nil, errors.New("configured search URL is invalid")
		}
		params := parsed.Query()
		params.Set("q", query)
		parsed.RawQuery = params.Encode()
		endpoint = parsed.String()
	case setting.AssistantSearchProviderGenericHTTP:
		parsed, err := url.Parse(endpoint)
		if err != nil {
			return nil, errors.New("configured search URL is invalid")
		}
		params := parsed.Query()
		params.Set("q", query)
		parsed.RawQuery = params.Encode()
		endpoint = parsed.String()
	default:
		return nil, errors.New("configured search provider is invalid")
	}

	request, err := http.NewRequestWithContext(ctx, method, endpoint, body)
	if err != nil {
		return nil, errors.New("search request could not be created")
	}
	request.Header.Set("Accept", "application/json")
	if body != nil {
		request.Header.Set("Content-Type", "application/json")
	}
	switch provider {
	case setting.AssistantSearchProviderExa:
		request.Header.Set("x-api-key", apiKey)
	case setting.AssistantSearchProviderTavily:
		request.Header.Set("Authorization", "Bearer "+apiKey)
	case setting.AssistantSearchProviderBrave:
		request.Header.Set("X-Subscription-Token", apiKey)
	case setting.AssistantSearchProviderGenericHTTP:
		if apiKey != "" {
			// Preserve the old adapter's behavior for existing deployments.
			request.Header.Set("Authorization", "Bearer "+apiKey)
			request.Header.Set("X-API-Key", apiKey)
		}
	}
	return request, nil
}

func newAssistantSearchHTTPClient() *http.Client {
	return newAssistantSearchHTTPClientWithTransport(&http.Transport{
		Proxy: nil,
		DialContext: func(ctx context.Context, network, address string) (net.Conn, error) {
			return dialAssistantSearchProviderAddress(ctx, network, address)
		},
	})
}

func newAssistantSearchHTTPClientWithTransport(transport http.RoundTripper) *http.Client {
	if transport == nil {
		transport = http.DefaultTransport
	}
	return &http.Client{
		Timeout:   assistantSearchDefaultTimeout,
		Transport: assistantSearchResponseLimitTransport{base: transport},
		CheckRedirect: func(request *http.Request, via []*http.Request) error {
			if len(via) >= assistantSearchMaxRedirects ||
				setting.ValidateAssistantSearchURL(request.URL.String()) != nil ||
				len(via) == 0 || !sameAssistantSearchOrigin(via[0].URL, request.URL) {
				return errors.New("search provider redirect is not allowed")
			}
			return nil
		},
	}
}

func sameAssistantSearchOrigin(first, next *url.URL) bool {
	if first == nil || next == nil || !strings.EqualFold(first.Scheme, next.Scheme) ||
		!strings.EqualFold(first.Hostname(), next.Hostname()) {
		return false
	}
	effectivePort := func(value *url.URL) string {
		if port := value.Port(); port != "" {
			return port
		}
		if strings.EqualFold(value.Scheme, "https") {
			return "443"
		}
		if strings.EqualFold(value.Scheme, "http") {
			return "80"
		}
		return ""
	}
	return effectivePort(first) == effectivePort(next)
}

func dialAssistantSearchProviderAddress(ctx context.Context, network, address string) (net.Conn, error) {
	host, port, err := net.SplitHostPort(address)
	if err != nil {
		return nil, errors.New("search provider address is invalid")
	}
	dialer := &net.Dialer{Timeout: assistantSearchDefaultTimeout}
	if ip := net.ParseIP(host); ip != nil {
		if !setting.IsAssistantSearchPublicIP(ip) {
			return nil, errors.New("search provider resolved to a non-public address")
		}
		return dialer.DialContext(ctx, network, net.JoinHostPort(ip.String(), port))
	}
	ips, err := net.DefaultResolver.LookupIP(ctx, "ip", host)
	if err != nil {
		return nil, errors.New("search provider hostname could not be resolved")
	}
	var lastErr error
	for _, ip := range ips {
		if !setting.IsAssistantSearchPublicIP(ip) {
			continue
		}
		connection, dialErr := dialer.DialContext(ctx, network, net.JoinHostPort(ip.String(), port))
		if dialErr == nil {
			return connection, nil
		}
		lastErr = dialErr
	}
	if lastErr != nil {
		return nil, errors.New("search provider has no reachable public address")
	}
	return nil, errors.New("search provider resolved only to non-public addresses")
}

func readAssistantSearchResponse(body io.Reader) ([]byte, error) {
	data, err := common.ReadAllLimit(body, assistantSearchMaxResponseBytes)
	if errors.Is(err, common.ErrLimitExceeded) {
		return nil, errors.New("search provider response is too large")
	}
	return data, err
}

type assistantSearchResponseLimitTransport struct {
	base http.RoundTripper
}

func (transport assistantSearchResponseLimitTransport) RoundTrip(request *http.Request) (*http.Response, error) {
	response, err := transport.base.RoundTrip(request)
	if err != nil || response == nil || response.Body == nil {
		return response, err
	}
	response.Body = &assistantSearchLimitedBody{ReadCloser: response.Body, remaining: assistantSearchMaxResponseBytes}
	return response, nil
}

type assistantSearchLimitedBody struct {
	io.ReadCloser
	remaining int64
}

func (body *assistantSearchLimitedBody) Read(p []byte) (int, error) {
	if body.remaining <= 0 {
		var probe [1]byte
		n, err := body.ReadCloser.Read(probe[:])
		if n > 0 {
			return 0, errors.New("search provider response is too large")
		}
		return 0, err
	}
	if int64(len(p)) > body.remaining {
		p = p[:body.remaining]
	}
	n, err := body.ReadCloser.Read(p)
	body.remaining -= int64(n)
	return n, err
}

type assistantSearchBearerTransport struct {
	base   http.RoundTripper
	token  string
	origin *url.URL
}

func (transport assistantSearchBearerTransport) RoundTrip(request *http.Request) (*http.Response, error) {
	clone := request.Clone(request.Context())
	clone.Header = request.Header.Clone()
	if token := strings.TrimSpace(transport.token); token != "" && sameAssistantSearchOrigin(transport.origin, request.URL) {
		clone.Header.Set("Authorization", "Bearer "+token)
	}
	return transport.base.RoundTrip(clone)
}

func executeAssistantMCP(ctx context.Context, endpoint, apiKey, configuredTool, query string, client *http.Client) (any, error) {
	if client == nil {
		client = newAssistantSearchHTTPClient()
	}
	if client.Transport == nil {
		client.Transport = http.DefaultTransport
	}
	mcpClient := *client
	origin, err := url.Parse(endpoint)
	if err != nil {
		return nil, errors.New("MCP search endpoint is invalid")
	}
	mcpClient.Transport = assistantSearchBearerTransport{base: client.Transport, token: apiKey, origin: origin}

	clientSDK := mcp.NewClient(&mcp.Implementation{
		Name:    "api.lmm.best-assistant-search",
		Version: "1.0.0",
	}, &mcp.ClientOptions{
		MultiRoundTrip: &mcp.MultiRoundTripOptions{Disabled: true},
	})
	session, err := clientSDK.Connect(ctx, &mcp.StreamableClientTransport{
		Endpoint:             endpoint,
		HTTPClient:           &mcpClient,
		MaxRetries:           -1,
		DisableStandaloneSSE: true,
	}, nil)
	if err != nil {
		return nil, errors.New("MCP search server could not be connected")
	}
	defer session.Close()

	tools, err := session.ListTools(ctx, nil)
	if err != nil {
		return nil, errors.New("MCP search tools could not be listed")
	}
	tool, err := selectAssistantMCPTool(tools.Tools, configuredTool)
	if err != nil {
		return nil, err
	}
	argumentName := assistantMCPQueryArgument(tool)
	result, err := session.CallTool(ctx, &mcp.CallToolParams{
		Name:      tool.Name,
		Arguments: map[string]any{argumentName: query},
	})
	if err != nil {
		return nil, errors.New("MCP search tool request failed")
	}
	return assistantMCPResultData(result)
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
		bounded, err := common.MarshalLimit(result.StructuredContent, assistantSearchMaxResponseBytes)
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
			if len(text.Text) > assistantSearchMaxResponseBytes {
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
	bounded, err := common.MarshalLimit(texts, assistantSearchMaxResponseBytes)
	if err != nil {
		return nil, errors.New("MCP search result is too large")
	}
	var decoded []string
	if err := json.Unmarshal(bounded, &decoded); err != nil {
		return nil, errors.New("MCP search result is invalid")
	}
	return decoded, nil
}
