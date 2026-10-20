// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package toolmarket

import (
	"encoding/json"
	"strings"
	"testing"
)

func TestLegacyExecutionBuiltinPreviewRemovesPrivateTokensFromBothRepresentations(t *testing.T) {
	r := LegacyExecution{Builtin: &LegacyBuiltinExecution{Continuation: json.RawMessage(`{"operation_id":"original","confirmation":"private-opaque"}`), PreviewResult: json.RawMessage(`{"content":[{"type":"text","text":"private-opaque"}],"structuredContent":{"receipt":{"confirmation":"private-opaque","confirmation_url":"https://core.example/core/v1/bounties/confirm/original"},"summary":"cost private-opaque","output":{"b64_json":"image"},"operation_id":"original"}}`)}}
	got := LegacyBuiltinPublicPreview(r)
	if len(got) == 0 || strings.Contains(string(got), "private-opaque") || strings.Contains(string(got), "b64_json") || strings.Contains(string(got), `"output"`) || !strings.Contains(string(got), "https://core.example/core/v1/bounties/confirm/original") || !strings.Contains(string(got), "original") {
		t.Fatalf("unsafe preview %s", got)
	}
	var envelope map[string]any
	if json.Unmarshal(got, &envelope) != nil || envelope["content"] == nil || envelope["structuredContent"] == nil {
		t.Fatal("invalid envelope")
	}
}
