// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package webcompat

import (
	"context"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/toolmarket"
)

type marketRecoveryResponseFixture struct{ marketTransportFake }

func (f *marketRecoveryResponseFixture) LegacyWeb(ctx context.Context, token, account string, in toolmarket.WebRequest) (any, error) {
	f.marketTransportFake.LegacyWeb(ctx, token, account, in)
	return toolmarket.LegacyWebExecutionResponse{Call: map[string]any{"id": "original-call", "execution_status": "unknown", "settlement_status": "reserved"}, RequestState: strings.Repeat("a", 64), ErrorCode: "TOOL_MARKET_EXECUTION_PENDING"}, toolmarket.ErrPending
}
func TestMarketRecoveryPendingAlwaysRetainsOriginalReceipt(t *testing.T) {
	for _, tc := range []struct{ method, path, body, permission string }{
		{"GET", "/api/tool-market/calls/original-call/result", "", "toolmarket.read"},
		{"GET", "/api/tool-market/executions/by-request/original-key", "", "toolmarket.read"},
		{"POST", "/api/tool-market/calls/original-call/resume", `{"request_state":"` + strings.Repeat("a", 64) + `"}`, "toolmarket.execute"},
	} {
		f := &marketRecoveryResponseFixture{}
		rec := httptest.NewRecorder()
		marketMux(t, &authCore{}, f).ServeHTTP(rec, marketHTTP(tc.method, tc.path, tc.body, true))
		body := marketEnvelope(t, rec)
		data, ok := body["data"].(map[string]any)
		if rec.Code != 409 || body["success"] != false || body["code"] != "TOOL_MARKET_EXECUTION_PENDING" || !ok || f.permission != tc.permission {
			t.Fatal("pending receipt discarded or unauthorized", rec.Code, body)
		}
		call, ok := data["call"].(map[string]any)
		if !ok || call["id"] != "original-call" || data["request_state"] != strings.Repeat("a", 64) {
			t.Fatal("original receipt changed", body)
		}
	}
}
