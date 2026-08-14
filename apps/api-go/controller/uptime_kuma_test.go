package controller

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/gin-gonic/gin"
)

type uptimeStatusResponse struct {
	Success  bool                `json:"success"`
	Degraded bool                `json:"degraded"`
	Message  string              `json:"message"`
	Data     []UptimeGroupResult `json:"data"`
}

func uptimeTestGroup(serverURL, categoryName string) map[string]interface{} {
	return map[string]interface{}{
		"url":          serverURL,
		"slug":         "public",
		"categoryName": categoryName,
	}
}

func serveUptimeTestRequest(
	t *testing.T,
	groups []map[string]interface{},
	client *http.Client,
	timeout time.Duration,
) (*httptest.ResponseRecorder, uptimeStatusResponse) {
	t.Helper()
	gin.SetMode(gin.TestMode)
	recorder := httptest.NewRecorder()
	ctx, _ := gin.CreateTestContext(recorder)
	ctx.Request = httptest.NewRequest(http.MethodGet, "/api/uptime/status", nil)

	serveUptimeKumaStatus(ctx, groups, client, timeout)

	var response uptimeStatusResponse
	if err := json.Unmarshal(recorder.Body.Bytes(), &response); err != nil {
		t.Fatalf("decode uptime response: %v; body=%q", err, recorder.Body.String())
	}
	return recorder, response
}

func writeUptimeSuccess(w http.ResponseWriter, path string) {
	w.Header().Set("Content-Type", "application/json")
	switch {
	case strings.HasPrefix(path, apiHeartbeatPath):
		_, _ = w.Write([]byte(`{"heartbeatList":{"7":[{"status":1}]},"uptimeList":{"7_24":0.999}}`))
	default:
		_, _ = w.Write([]byte(`{"publicGroupList":[{"id":1,"name":"Core","monitorList":[{"id":7,"name":"API"}]}]}`))
	}
}

func TestServeUptimeKumaStatusSuccessKeepsExistingShape(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		writeUptimeSuccess(w, r.URL.Path)
	}))
	defer server.Close()

	recorder, response := serveUptimeTestRequest(
		t,
		[]map[string]interface{}{uptimeTestGroup(server.URL, "Production")},
		server.Client(),
		time.Second,
	)

	if recorder.Code != http.StatusOK || !response.Success || response.Degraded {
		t.Fatalf("unexpected success response: status=%d response=%+v", recorder.Code, response)
	}
	if len(response.Data) != 1 || response.Data[0].CategoryName != "Production" || response.Data[0].Degraded {
		t.Fatalf("unexpected group response: %+v", response.Data)
	}
	if len(response.Data[0].Monitors) != 1 {
		t.Fatalf("expected one monitor, got %+v", response.Data[0].Monitors)
	}
	monitor := response.Data[0].Monitors[0]
	if monitor.Name != "API" || monitor.Group != "Core" || monitor.Status != 1 || monitor.Uptime != 0.999 {
		t.Fatalf("unexpected monitor: %+v", monitor)
	}
	if strings.Contains(recorder.Body.String(), `"degraded"`) || strings.Contains(recorder.Body.String(), `"error_code"`) {
		t.Fatalf("successful response shape should not gain degraded fields: %s", recorder.Body.String())
	}
}

func TestServeUptimeKumaStatusReportsUpstreamFailures(t *testing.T) {
	tests := []struct {
		name           string
		handler        http.HandlerFunc
		requestTimeout time.Duration
		wantErrorCode  string
		maxElapsed     time.Duration
	}{
		{
			name: "non-2xx",
			handler: func(w http.ResponseWriter, _ *http.Request) {
				http.Error(w, "unavailable", http.StatusServiceUnavailable)
			},
			requestTimeout: time.Second,
			wantErrorCode:  "upstream_http_error",
		},
		{
			name: "invalid JSON",
			handler: func(w http.ResponseWriter, _ *http.Request) {
				_, _ = w.Write([]byte(`{"broken":`))
			},
			requestTimeout: time.Second,
			wantErrorCode:  "invalid_upstream_response",
		},
		{
			name: "missing expected schema",
			handler: func(w http.ResponseWriter, _ *http.Request) {
				_, _ = w.Write([]byte(`{}`))
			},
			requestTimeout: time.Second,
			wantErrorCode:  "invalid_upstream_response",
		},
		{
			name: "timeout",
			handler: func(w http.ResponseWriter, _ *http.Request) {
				time.Sleep(200 * time.Millisecond)
				_, _ = w.Write([]byte(`{}`))
			},
			requestTimeout: 25 * time.Millisecond,
			wantErrorCode:  "upstream_timeout",
			maxElapsed:     150 * time.Millisecond,
		},
		{
			name: "oversized chunked response",
			handler: func(w http.ResponseWriter, _ *http.Request) {
				w.Header().Set("Content-Type", "application/json")
				w.(http.Flusher).Flush()
				_, _ = w.Write([]byte(`{"padding":"`))
				_, _ = w.Write([]byte(strings.Repeat("x", uptimeResponseMaxBytes)))
				_, _ = w.Write([]byte(`"}`))
			},
			requestTimeout: time.Second,
			wantErrorCode:  "upstream_response_too_large",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			server := httptest.NewServer(tt.handler)
			defer server.Close()

			started := time.Now()
			recorder, response := serveUptimeTestRequest(
				t,
				[]map[string]interface{}{uptimeTestGroup(server.URL, "Production")},
				server.Client(),
				tt.requestTimeout,
			)
			elapsed := time.Since(started)

			if recorder.Code != http.StatusBadGateway || response.Success || !response.Degraded {
				t.Fatalf("failure was not surfaced: status=%d response=%+v", recorder.Code, response)
			}
			if len(response.Data) != 1 || !response.Data[0].Degraded || response.Data[0].ErrorCode != tt.wantErrorCode {
				t.Fatalf("unexpected degraded group: %+v", response.Data)
			}
			if len(response.Data[0].Monitors) != 0 {
				t.Fatalf("failed upstream must not synthesize monitors: %+v", response.Data[0].Monitors)
			}
			if strings.Contains(recorder.Body.String(), server.URL) {
				t.Fatalf("response leaked upstream URL: %s", recorder.Body.String())
			}
			if tt.maxElapsed > 0 && elapsed > tt.maxElapsed {
				t.Fatalf("request timeout was not enforced: elapsed=%v max=%v", elapsed, tt.maxElapsed)
			}
		})
	}
}

func TestServeUptimeKumaStatusKeepsHealthyGroupsWhenPartiallyDegraded(t *testing.T) {
	healthy := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		writeUptimeSuccess(w, r.URL.Path)
	}))
	defer healthy.Close()
	unhealthy := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		http.Error(w, "unavailable", http.StatusBadGateway)
	}))
	defer unhealthy.Close()

	recorder, response := serveUptimeTestRequest(
		t,
		[]map[string]interface{}{
			uptimeTestGroup(healthy.URL, "Healthy"),
			uptimeTestGroup(unhealthy.URL, "Degraded"),
		},
		&http.Client{Timeout: time.Second},
		time.Second,
	)

	if recorder.Code != http.StatusOK || response.Success || !response.Degraded {
		t.Fatalf("partial failure must be a compatible degraded 200: status=%d response=%+v", recorder.Code, response)
	}
	if len(response.Data) != 2 || len(response.Data[0].Monitors) != 1 {
		t.Fatalf("healthy data was lost during partial failure: %+v", response.Data)
	}
	if response.Data[1].ErrorCode != "upstream_http_error" || !response.Data[1].Degraded {
		t.Fatalf("failed group was not marked degraded: %+v", response.Data[1])
	}
}
