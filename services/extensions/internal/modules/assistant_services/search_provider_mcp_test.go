package assistant_services

import (
	"context"
	"encoding/json"
	"github.com/modelcontextprotocol/go-sdk/mcp"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync/atomic"
	"testing"
	"time"
)

func TestMCPSearchOfficialServer(t *testing.T) {
	var calls, gets atomic.Int32
	sdk := mcp.NewServer(&mcp.Implementation{Name: "synthetic-search", Version: "1"}, nil)
	mcp.AddTool(sdk, &mcp.Tool{Name: "web_search", Description: "Search synthetic local data"}, func(ctx context.Context, request *mcp.CallToolRequest, input struct {
		Query string `json:"query"`
	}) (*mcp.CallToolResult, any, error) {
		calls.Add(1)
		return &mcp.CallToolResult{Content: []mcp.Content{&mcp.TextContent{Text: `{"query":"` + input.Query + `","echo":"private-test-key"}`}}}, nil, nil
	})
	handler := mcp.NewStreamableHTTPHandler(func(*http.Request) *mcp.Server { return sdk }, nil)
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method == http.MethodGet {
			gets.Add(1)
		}
		if r.Header.Get("Authorization") != "Bearer private-test-key" {
			t.Error("missing host credential")
			w.WriteHeader(401)
			return
		}
		handler.ServeHTTP(w, r)
	}))
	defer server.Close()
	provider, err := NewSearchProvider(SearchConfig{Provider: "mcp_streamable_http", Endpoint: server.URL, APIKey: "private-test-key", LocalHTTP: true, Timeout: 2 * time.Second})
	if err != nil {
		t.Fatal(err)
	}
	raw, err := provider.Search(context.Background(), "synthetic query")
	if err != nil {
		t.Fatal(err)
	}
	if calls.Load() != 1 || gets.Load() != 0 {
		t.Fatalf("calls=%d standaloneGET=%d", calls.Load(), gets.Load())
	}
	if strings.Contains(string(raw), "private-test-key") || !strings.Contains(string(raw), "[REDACTED]") {
		t.Fatalf("secret redaction failed: %s", raw)
	}
	var object map[string]any
	if json.Unmarshal(raw, &object) != nil || object["ok"] != true {
		t.Fatalf("invalid response: %s", raw)
	}
}
func TestMCPSearchNoRetryAndBounds(t *testing.T) {
	for _, tc := range []struct {
		name   string
		status int
		body   string
	}{{"unavailable", 503, "fail"}, {"oversized", 200, strings.Repeat("x", 65537)}, {"redirect", 302, ""}} {
		t.Run(tc.name, func(t *testing.T) {
			var requests atomic.Int32
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				requests.Add(1)
				w.Header().Set("Content-Type", "application/json")
				if tc.status == 302 {
					w.Header().Set("Location", "http://127.0.0.1:1/private")
				}
				w.WriteHeader(tc.status)
				w.Write([]byte(tc.body))
			}))
			defer server.Close()
			provider, err := NewSearchProvider(SearchConfig{Provider: "mcp_streamable_http", Endpoint: server.URL, LocalHTTP: true, Timeout: time.Second})
			if err != nil {
				t.Fatal(err)
			}
			_, err = provider.Search(context.Background(), "synthetic query")
			typed, ok := err.(*SearchError)
			if !ok || !typed.DispatchUnknown() {
				t.Fatalf("expected UNKNOWN: %v", err)
			}
			if requests.Load() != 1 {
				t.Fatalf("retried %d requests", requests.Load())
			}
		})
	}
}
