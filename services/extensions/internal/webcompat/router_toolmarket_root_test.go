package webcompat

import (
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

// Real Mount and HTTP transport; the domain port only records dispatch. Core
// funds, catalogue storage and runtime readiness are independently verified.
func TestMountToolMarketRootPreservesExactDispatchAndAdmission(t *testing.T) {
	host, core := mountTransportHost(t, nil)
	market := &marketTransportFake{}
	server := httptest.NewServer(Mount(host, newAuth(t, core), nil, nil, AdditionalServices{ToolMarket: market}))
	defer server.Close()
	request := func(method, path, authorization string) (int, string, http.Header) {
		t.Helper()
		req, err := http.NewRequest(method, server.URL+path, nil)
		if err != nil {
			t.Fatal(err)
		}
		if authorization != "" {
			req.Header.Set("Authorization", "Bearer "+authorization)
		}
		response, err := server.Client().Do(req)
		if err != nil {
			t.Fatal(err)
		}
		defer response.Body.Close()
		body, err := io.ReadAll(io.LimitReader(response.Body, 65536))
		if err != nil {
			t.Fatal(err)
		}
		return response.StatusCode, string(body), response.Header
	}
	root := "/api/tool-market?execution_type=builtin&q=&offset=0&limit=30"
	if code, _, headers := request("GET", root, webToken); code != 503 || headers.Get("Retry-After") != "1" || market.businessCalls != 0 || core.calls != 0 {
		t.Fatal("preparing root bypassed admission", code)
	}
	if err := host.MarkReady(); err != nil {
		t.Fatal(err)
	}
	for _, token := range []string{"", webToken} {
		if code, body, _ := request("GET", root, token); code != 200 || !strings.Contains(body, `"data":[]`) {
			t.Fatal("registered catalogue root unreachable", code, body)
		}
		if market.query.ExecutionType != "builtin" || market.query.Offset != 0 || market.query.Limit != 30 {
			t.Fatal("root query not forwarded")
		}
	}
	if market.token != webToken || market.account != "17" || core.calls != 1 {
		t.Fatal("root changed original user scope")
	}
	if code, _, _ := request("GET", "/api/tool-market/config", ""); code != 200 {
		t.Fatal("existing child path changed", code)
	}
	calls := market.businessCalls
	for _, path := range []string{"/api/tool-marketfoo", "/api/tool-marketfoo/config", "/api/tool-markets"} {
		if code, _, _ := request("GET", path, ""); code != 404 || market.businessCalls != calls {
			t.Fatal("adjacent path captured", path, code)
		}
	}
	if code, _, _ := request("GET", root, hostToken); code != 401 || market.businessCalls != calls {
		t.Fatal("host credential became Web user", code)
	}
	if code, _, _ := request("POST", "/api/tool-market", webToken); code < 400 || market.businessCalls != calls {
		t.Fatal("unsupported root method reached catalogue", code)
	}
	host.BeginDrain()
	if code, _, headers := request("GET", root, webToken); code != 503 || headers.Get("Retry-After") != "1" || market.businessCalls != calls {
		t.Fatal("draining root bypassed admission", code)
	}
}
