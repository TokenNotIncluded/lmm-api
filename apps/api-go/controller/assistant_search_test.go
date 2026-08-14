package controller

import (
	"context"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"testing"

	"github.com/LIghtJUNction/api.lmm.best/setting"
	"github.com/modelcontextprotocol/go-sdk/mcp"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestAssistantSearchProviderOptionValidation(t *testing.T) {
	providers := []string{"none", "exa", "tavily", "brave", "generic_http", "mcp_streamable_http"}
	for _, provider := range providers {
		assert.NoError(t, setting.ValidateAssistantOption(setting.AssistantSearchProviderOptionKey, provider), provider)
	}
	assert.Error(t, setting.ValidateAssistantOption(setting.AssistantSearchProviderOptionKey, "google"))
	assert.NoError(t, setting.ValidateAssistantOption(setting.AssistantSearchMCPToolOptionKey, ""))
	assert.NoError(t, setting.ValidateAssistantOption(setting.AssistantSearchMCPToolOptionKey, "web_search"))
	assert.Error(t, setting.ValidateAssistantOption(setting.AssistantSearchMCPToolOptionKey, strings.Repeat("x", 129)))
}

func TestBuildAssistantSearchRequestProtocols(t *testing.T) {
	tests := []struct {
		name           string
		settings       setting.AssistantSettings
		method         string
		wantURLPart    string
		wantHeader     string
		wantHeaderVal  string
		wantBodyValues map[string]any
	}{
		{
			name: "exa",
			settings: setting.AssistantSettings{
				SearchProvider: setting.AssistantSearchProviderExa,
				SearchAPIKey:   "exa-secret",
			},
			method:        http.MethodPost,
			wantURLPart:   "https://api.exa.ai/search",
			wantHeader:    "x-api-key",
			wantHeaderVal: "exa-secret",
			wantBodyValues: map[string]any{
				"query": "latest Go release",
				"contents": map[string]any{
					"highlights": true,
				},
			},
		},
		{
			name: "tavily",
			settings: setting.AssistantSettings{
				SearchProvider: setting.AssistantSearchProviderTavily,
				SearchAPIKey:   "tavily-secret",
			},
			method:        http.MethodPost,
			wantURLPart:   "https://api.tavily.com/search",
			wantHeader:    "Authorization",
			wantHeaderVal: "Bearer tavily-secret",
			wantBodyValues: map[string]any{
				"query": "latest Go release",
			},
		},
		{
			name: "brave",
			settings: setting.AssistantSettings{
				SearchProvider: setting.AssistantSearchProviderBrave,
				SearchAPIKey:   "brave-secret",
				SearchURL:      "https://search.example.test/web",
			},
			method:        http.MethodGet,
			wantURLPart:   "https://search.example.test/web?q=latest+Go+release",
			wantHeader:    "X-Subscription-Token",
			wantHeaderVal: "brave-secret",
		},
		{
			name: "generic HTTP",
			settings: setting.AssistantSettings{
				SearchProvider: setting.AssistantSearchProviderGenericHTTP,
				SearchAPIKey:   "generic-secret",
				SearchURL:      "https://search.example.test/api/search?lang=en",
			},
			method:        http.MethodGet,
			wantURLPart:   "https://search.example.test/api/search?lang=en&q=latest+Go+release",
			wantHeader:    "Authorization",
			wantHeaderVal: "Bearer generic-secret",
		},
	}

	for _, testCase := range tests {
		t.Run(testCase.name, func(t *testing.T) {
			request, err := buildAssistantSearchRequest(context.Background(), testCase.settings, " latest Go release ")
			require.NoError(t, err)
			assert.Equal(t, testCase.method, request.Method)
			assert.Equal(t, testCase.wantURLPart, request.URL.String())
			assert.Equal(t, testCase.wantHeaderVal, request.Header.Get(testCase.wantHeader))

			if testCase.wantBodyValues == nil {
				return
			}
			body, err := io.ReadAll(request.Body)
			require.NoError(t, err)
			var values map[string]any
			require.NoError(t, json.Unmarshal(body, &values))
			assert.Equal(t, testCase.wantBodyValues, values)
		})
	}
}

func TestAssistantSearchExecutionUsesBoundedProviderResponse(t *testing.T) {
	var captured *http.Request
	transport := assistantSearchRoundTripFunc(func(request *http.Request) (*http.Response, error) {
		captured = request.Clone(request.Context())
		return &http.Response{
			StatusCode: http.StatusOK,
			Body:       io.NopCloser(strings.NewReader(`{"results":[{"title":"Go"}]}`)),
			Header:     make(http.Header),
			Request:    request,
		}, nil
	})
	client := newAssistantSearchHTTPClientWithTransport(transport)
	response, err := executeAssistantSearch(context.Background(), setting.AssistantSettings{
		SearchProvider: setting.AssistantSearchProviderGenericHTTP,
		SearchURL:      "https://search.example.test/api/search",
		SearchAPIKey:   "server-only-secret",
	}, "Go", client)
	require.NoError(t, err)
	assert.True(t, response.Configured)
	assert.Equal(t, "Go", response.Query)
	assert.Equal(t, http.StatusOK, response.Status)
	assert.NotNil(t, response.Results)
	require.NotNil(t, captured)
	assert.Equal(t, "Go", captured.URL.Query().Get("q"))
	assert.Equal(t, "Bearer server-only-secret", captured.Header.Get("Authorization"))
}

func TestAssistantSearchRedirectsAndBearerCredentialsStayOnOrigin(t *testing.T) {
	origin, err := url.Parse("https://mcp.example.test/search")
	require.NoError(t, err)
	redirectSame, err := url.Parse("https://mcp.example.test/next")
	require.NoError(t, err)
	redirectOther, err := url.Parse("https://attacker.example.test/next")
	require.NoError(t, err)

	client := newAssistantSearchHTTPClientWithTransport(assistantSearchRoundTripFunc(func(request *http.Request) (*http.Response, error) {
		return &http.Response{StatusCode: http.StatusOK, Body: io.NopCloser(strings.NewReader(`{}`)), Header: make(http.Header)}, nil
	}))
	assert.NoError(t, client.CheckRedirect(&http.Request{URL: redirectSame}, []*http.Request{{URL: origin}}))
	assert.Error(t, client.CheckRedirect(&http.Request{URL: redirectOther}, []*http.Request{{URL: origin}}))

	var captured http.Header
	transport := assistantSearchBearerTransport{
		origin: origin,
		token:  "server-only-secret",
		base: assistantSearchRoundTripFunc(func(request *http.Request) (*http.Response, error) {
			captured = request.Header.Clone()
			return &http.Response{StatusCode: http.StatusOK, Body: io.NopCloser(strings.NewReader(`{}`)), Header: make(http.Header)}, nil
		}),
	}
	request := &http.Request{Method: http.MethodGet, URL: redirectOther, Header: make(http.Header)}
	_, err = transport.RoundTrip(request)
	require.NoError(t, err)
	assert.Empty(t, captured.Get("Authorization"))
	request.URL = redirectSame
	_, err = transport.RoundTrip(request)
	require.NoError(t, err)
	assert.Equal(t, "Bearer server-only-secret", captured.Get("Authorization"))
}

func TestSelectAssistantMCPTool(t *testing.T) {
	tools := []*mcp.Tool{
		{Name: "fetch_page"},
		{Name: "web_search_exa"},
		{Name: "search"},
		{Name: "web_search"},
	}

	selected, err := selectAssistantMCPTool(tools, "")
	require.NoError(t, err)
	assert.Equal(t, "web_search", selected.Name)

	selected, err = selectAssistantMCPTool(tools, "web_search_exa")
	require.NoError(t, err)
	assert.Equal(t, "web_search_exa", selected.Name)

	_, err = selectAssistantMCPTool(tools, "missing_search")
	assert.Error(t, err)

	_, err = selectAssistantMCPTool([]*mcp.Tool{{Name: "fetch_page"}}, "")
	assert.Error(t, err)
}

type assistantSearchRoundTripFunc func(*http.Request) (*http.Response, error)

func (roundTrip assistantSearchRoundTripFunc) RoundTrip(request *http.Request) (*http.Response, error) {
	return roundTrip(request)
}

func TestAssistantSearchRedirectPolicyRejectsPrivateLiteral(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		http.Redirect(writer, request, "http://127.0.0.1/private", http.StatusFound)
	}))
	t.Cleanup(server.Close)

	client := newAssistantSearchHTTPClient()
	_, err := client.Get(server.URL)
	assert.Error(t, err)
}
