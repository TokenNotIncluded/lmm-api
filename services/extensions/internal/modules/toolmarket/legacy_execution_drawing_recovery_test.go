// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package toolmarket

import (
	"context"
	"encoding/json"
	"errors"
	"testing"
)

func retiredDrawingFixture(t *testing.T) (*Module, *executionAuthority, *executionBuiltinRuntime, LegacyExecution) {
	t.Helper()
	m, port, runtime, input := webNamedBuiltinRecoveryFixture(t, "drawing", "drawing.generate", json.RawMessage(`{"prompt":"original"}`))
	runtime.result.DownstreamOperationID = "image-original"
	runtime.result.Continuation = json.RawMessage(`{"operation_id":"image-original"}`)
	r, err := m.LegacyExecute(context.Background(), "1", "1", input)
	if !errors.Is(err, ErrPending) {
		t.Fatal(err)
	}
	definition, tool, err := drawingV1RecoveryDefinition("37ff72e9-6f8e-5e34-a8e1-988253f20d27", "23ff32df-37fb-5105-9063-db1228dd3d3b")
	if err != nil {
		t.Fatal(err)
	}
	r.Input.VersionID = definition.VersionID
	r.RequestHash = digest(r.Input)
	r.Core.VersionID = definition.VersionID
	r.Builtin.InputSchemaDigest = tool.InputSchemaDigest
	r.MeteringBinding.VersionID = definition.VersionID
	r.MeteringBinding.RemoteDigest = tool.InputSchemaDigest
	port.r = r.Core
	if err = m.store.update(func(d *database) error {
		d.LegacyExecutions[r.ID] = r
		var tools []WebTool
		remote := map[string]string{}
		for _, entry := range definition.Tools {
			tools = append(tools, entry.WebTool)
			remote[entry.ToolID] = entry.InputSchemaDigest
		}
		d.Web.Versions[definition.VersionID] = webSnapshot{Version: WebVersion{ID: definition.VersionID, ServiceID: definition.ServiceID, Digest: definition.Digest, ExecutionType: "builtin"}, Tools: tools, Digest: definition.Digest, ValidationDigest: definition.Digest, RemoteDigests: remote}
		return nil
	}); err != nil {
		t.Fatal(err)
	}
	return m, port, runtime, r
}

func TestDrawingRetiredVersionRecoveryRequiresOriginalDurableSnapshot(t *testing.T) {
	m, port, runtime, r := retiredDrawingFixture(t)
	ctx, _, _, err := m.builtinRecoveryContext(context.Background(), r)
	if err != nil {
		t.Fatal("original retained snapshot rejected", err)
	}
	in := legacyBuiltinInvocation(r, "")
	if _, _, err = drawingRecoveryInvocation(context.Background(), in); err == nil {
		t.Fatal("caller version selected recovery without durable permit")
	}
	if _, _, err = drawingRecoveryInvocation(ctx, in); err != nil {
		t.Fatal(err)
	}
	for _, field := range []string{"call", "actor", "input", "grant", "version"} {
		changed := in
		switch field {
		case "call":
			changed.Context.CallID = "foreign"
		case "actor":
			changed.Owner = Principal{User: "2", Account: "2"}
		case "input":
			changed.Arguments = json.RawMessage(`{"prompt":"replacement"}`)
		case "grant":
			changed.Context.GrantID = "foreign"
		case "version":
			changed.Context.VersionID = "foreign"
		}
		if _, _, err = drawingRecoveryInvocation(ctx, changed); err == nil {
			t.Fatal("retired permit changed original binding", field)
		}
	}
	if _, err = m.LegacyInspectExecution(context.Background(), "1", "1", r.ID); !errors.Is(err, ErrPending) || runtime.resumes != 0 || runtime.invokes != 1 {
		t.Fatal("retired GET executed model", err)
	}
	newInput := r.Input
	newInput.RequestKey = "fresh-old-version-call"
	if _, err = m.LegacyExecute(context.Background(), "1", "1", newInput); err == nil || port.reserves != 1 {
		t.Fatal("retired grant created a new call", err)
	}
	if err = m.store.update(func(d *database) error { delete(d.Web.Versions, r.Input.VersionID); return nil }); err != nil {
		t.Fatal(err)
	}
	if _, _, _, err = m.builtinRecoveryContext(context.Background(), r); err == nil {
		t.Fatal("missing original snapshot accepted")
	}
}

type retiredDrawingPort struct{ invokes, queries, resumes int }

func (p *retiredDrawingPort) InvokeDrawing(context.Context, string, BuiltinInvocation, string) (BuiltinResult, error) {
	p.invokes++
	return BuiltinResult{}, ErrDenied
}
func (p *retiredDrawingPort) QueryDrawing(_ context.Context, _ string, in BuiltinInvocation, id string) (BuiltinResult, error) {
	p.queries++
	return BuiltinResult{CallID: in.Context.CallID, DownstreamOperationID: id, ResumeReady: true, ExecutionStatus: "confirmed", SettlementStatus: "pending"}, nil
}
func (p *retiredDrawingPort) ResumeDrawing(_ context.Context, _ string, in BuiltinInvocation, id string) (BuiltinResult, error) {
	p.resumes++
	return BuiltinResult{CallID: in.Context.CallID, DownstreamOperationID: id, ExecutionStatus: "completed", SettlementStatus: "settled"}, nil
}

func TestDrawingRetiredDispatcherAllowsOnlyModuleMintedRecovery(t *testing.T) {
	m, _, _, r := retiredDrawingFixture(t)
	ctx, _, _, err := m.builtinRecoveryContext(context.Background(), r)
	if err != nil {
		t.Fatal(err)
	}
	port := &retiredDrawingPort{}
	gate := &builtinGateStub{}
	dispatcher := &BuiltinDispatcher{Gate: gate, Drawing: port}
	in := legacyBuiltinInvocation(r, "")
	if _, err = dispatcher.QueryBuiltin(context.Background(), "original-session", in); !errors.Is(err, ErrDenied) || port.queries != 0 {
		t.Fatal("old caller version selected query without retained-call permit", err)
	}
	if _, err = dispatcher.ResumeBuiltin(context.Background(), "original-session", in); !errors.Is(err, ErrDenied) || port.resumes != 0 {
		t.Fatal("old caller version selected resume without permit", err)
	}
	if _, err = dispatcher.QueryBuiltin(ctx, "original-session", in); err != nil || port.queries != 1 {
		t.Fatal("retained query refused", err)
	}
	if _, err = dispatcher.ResumeBuiltin(ctx, "original-session", in); err != nil || port.resumes != 1 {
		t.Fatal("retained explicit resume refused", err)
	}
	if _, err = dispatcher.InvokeBuiltin(ctx, "original-session", in); !errors.Is(err, ErrDenied) || port.invokes != 0 || gate.calls != 0 {
		t.Fatal("recovery permit opened a new old-version invoke", err)
	}
}
