package webcompat

import (
	"net/http"
	"testing"
)

func TestMarketMCPMountUsesGlobalAdmissionAndPreservesResourceBearer(t *testing.T) {
	host, core := mountTransportHost(t, nil)
	calls := 0
	mcp := http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		calls++
		if r.Header.Get("Authorization") != "Bearer resource-only" {
			t.Error("resource bearer was converted or stripped")
		}
		w.WriteHeader(204)
	})
	handler := Mount(host, newAuth(t, core), nil, nil, AdditionalServices{ToolMarketMCP: mcp})
	request := func() int {
		return mountTransportCall(handler, "GET", "/mcp/market", []string{"Bearer resource-only"}).Code
	}
	if request() != 503 || calls != 0 {
		t.Fatal("MCP bypassed preparing gate")
	}
	if err := host.MarkReady(); err != nil {
		t.Fatal(err)
	}
	if request() != 204 || calls != 1 || core.calls != 0 {
		t.Fatal("MCP entered Session or native host API", calls, core.calls)
	}
	host.BeginDrain()
	if request() != 503 || calls != 1 {
		t.Fatal("MCP bypassed draining gate")
	}
}
