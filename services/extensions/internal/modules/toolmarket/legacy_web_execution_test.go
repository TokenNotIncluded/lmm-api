// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package toolmarket

import (
	"encoding/json"
	"strings"
	"testing"
)

func TestLegacyWebExecutionDoesNotExposeInputsOrInventSettlement(t *testing.T) {
	m, _, _ := webFixture(t)
	r := LegacyExecution{ID: "call", Owner: Principal{User: "1", Account: "1"}, State: "unknown", Input: LegacyExecuteInput{ClientID: "web-market", RequestKey: "private-request", ToolID: "tool", VersionID: "version", GrantID: "grant", Arguments: json.RawMessage(`{"secret":"private-argument"}`)}, InputDigest: "private-digest", Result: json.RawMessage(`{"content":[]}`)}
	response, err := m.legacyWebExecution(r, true)
	if err != nil {
		t.Fatal(err)
	}
	if response.Call["execution_status"] != "unknown" || response.Call["settlement_status"] != "reserved" || response.ErrorCode != "TOOL_MARKET_EXECUTION_PENDING" {
		t.Fatal("unknown operation was presented as final")
	}
	if _, exists := response.Call["price_quota"]; exists {
		t.Fatal("unacknowledged reserve invented a price")
	}
	wire, _ := json.Marshal(response)
	for _, secret := range []string{"private-request", "private-argument", "private-digest"} {
		if strings.Contains(string(wire), secret) {
			t.Fatal("private execution input leaked")
		}
	}
	r.Core = LegacyCreditCall{ID: "call", State: "settled", Held: 12, Charged: 7, Fee: 2}
	r.Success, r.ResultDigest, r.State = true, "digest", "done"
	response, err = m.legacyWebExecution(r, false)
	if err != nil || response.Call["price_quota"] != CreditAmount(7) || response.Call["fee_quota"] != CreditAmount(2) || response.Call["settlement_status"] != "settled" || response.Call["execution_status"] != "succeeded" {
		t.Fatal("settled projection differs from original Core receipt", err)
	}
}
