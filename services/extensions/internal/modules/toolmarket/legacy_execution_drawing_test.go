// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package toolmarket

import (
	"context"
	"encoding/json"
	"errors"
	"strings"
	"testing"
)

func drawingTestResult(t *testing.T, size int) BuiltinResult {
	t.Helper()
	raw, err := json.Marshal(map[string]any{"content": []any{map[string]any{"type": "text", "text": "completed"}}, "structuredContent": map[string]any{"operation_id": "image-original", "state": "completed", "billing_pending": false, "charged_credits": 7, "output": map[string]any{"b64_json": strings.Repeat("x", size)}}})
	if err != nil {
		t.Fatal(err)
	}
	return BuiltinResult{DownstreamOperationID: "image-original", Continuation: json.RawMessage(`{"operation_id":"image-original"}`), Result: raw, ExecutionStatus: "completed", SettlementStatus: "settled"}
}
func TestLegacyExecutionDrawingLargeOutputReferenceSurvivesReopenAndQueryACKLoss(t *testing.T) {
	m, port, runtime, input := builtinNamedExecutionFixture(t, "drawing", "drawing.generate", json.RawMessage(`{"prompt":"test"}`))
	runtime.result = drawingTestResult(t, 3<<20)
	got, err := m.LegacyExecute(context.Background(), "1", "1", input)
	if err != nil || len(got.Result) < 3<<20 || got.State != "done" || !got.Builtin.ExternalResult {
		t.Fatalf("large output dropped: size=%d err=%v state=%s", len(got.Result), err, got.State)
	}
	if runtime.invokes != 1 || runtime.queries != 1 || port.reserves != 1 || port.finishes != 1 {
		t.Fatalf("wrong flow: %+v", runtime)
	}
	var snapshot []byte
	if err = m.store.view(func(d database) error { var e error; snapshot, e = json.Marshal(d); return e }); err != nil {
		t.Fatal(err)
	}
	if len(snapshot) > 128<<10 || strings.Contains(string(snapshot), strings.Repeat("x", 100)) {
		t.Fatal("image bytes persisted in snapshot")
	}
	var restored database
	if err = json.Unmarshal(snapshot, &restored); err != nil {
		t.Fatal(err)
	}
	reopened, err := New(&Store{data: restored}, port, Config{WebOnly: true, BuiltinRuntime: runtime})
	if err != nil {
		t.Fatal(err)
	}
	reopened.now = m.now
	runtime.queryErr = ErrUpstream
	pending, err := reopened.LegacyExecute(context.Background(), "1", "1", input)
	if !errors.Is(err, ErrPending) || len(pending.Result) != 0 || runtime.invokes != 1 {
		t.Fatal("lost query ACK replayed generation", err)
	}
	runtime.queryErr = nil
	got, err = reopened.LegacyExecute(context.Background(), "1", "1", input)
	if err != nil || len(got.Result) < 3<<20 || runtime.invokes != 1 || port.reserves != 1 {
		t.Fatal("reopened original output unavailable", err)
	}
	runtime.result = drawingTestResult(t, 3<<20+1)
	if _, err = reopened.LegacyExecute(context.Background(), "1", "1", input); !errors.Is(err, ErrDenied) || runtime.invokes != 1 {
		t.Fatal("changed Core result accepted", err)
	}
}
func TestLegacyExecutionDrawingLimitCannotBeSelectedByUncompiledBuiltin(t *testing.T) {
	m, _, runtime, input := builtinExecutionFixture(t)
	runtime.result = drawingTestResult(t, 3<<20)
	if _, err := m.LegacyExecute(context.Background(), "1", "1", input); !errors.Is(err, ErrDenied) {
		t.Fatal("non drawing gained large bound", err)
	}
	r := LegacyExecution{Builtin: &LegacyBuiltinExecution{ServiceKey: "drawing", ToolName: "drawing.generate"}}
	if legacyBuiltinResultLimit(r) != 2<<20 {
		t.Fatal("forged drawing flag expanded limit")
	}
}
