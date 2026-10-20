// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package toolmarket

import "testing"

func TestLegacyMCPSchemaAndDecoderShareClosedActions(t *testing.T) {
	if len(legacyMCPInputSchema()["oneOf"].([]any)) != len(legacyMCPActions) {
		t.Fatal("schema action drift")
	}
	good := []string{`{"action":"search"}`, `{"action":"status"}`, `{"action":"usage"}`, `{"action":"details","service_id":"s"}`, `{"action":"invoke","tool_id":"t","version_id":"v","request_id":"r","arguments":{"large":9007199254740991}}`, `{"action":"authorize","tool_id":"t","version_id":"v","max_price_quota":0,"max_total_quota":0,"max_calls":1,"expires_at":101}`}
	for _, s := range good {
		in, e := legacyDecodeMCPInput([]byte(s), 100)
		if e != nil {
			t.Fatal(s, e)
		}
		if in.Action == "search" && in.Limit != 5 || in.Action == "usage" && in.Limit != 20 {
			t.Fatal(in)
		}
	}
	bad := []string{`null`, `[]`, `{"action":"status","action":"invoke"}`, `{"action":"status","tool_id":"t"}`, `{"action":"search","query":null}`, `{"action":"search","limit":0}`, `{"action":"search","limit":1.0}`, `{"action":"invoke","tool_id":"t","version_id":"v","request_id":"r"}`, `{"action":"invoke","tool_id":"t","version_id":"v","request_id":"r","arguments":[]}`, `{"action":"set_client_budget","limit_quota":9007199254740992}`, `{"action":"authorize","tool_id":"t","version_id":"v","max_price_quota":0,"max_total_quota":0,"max_calls":1,"expires_at":100}`}
	for _, s := range bad {
		if _, e := legacyDecodeMCPInput([]byte(s), 100); e == nil {
			t.Fatal("accepted", s)
		}
	}
}
