// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package toolmarket

import (
	"context"
	"encoding/json"
	"errors"
	"testing"
)

func webBuiltinRecoveryFixture(t *testing.T) (*Module, *executionAuthority, *executionBuiltinRuntime, LegacyExecuteInput) {
	return webNamedBuiltinRecoveryFixture(t, "wallet", "wallet.balance", json.RawMessage(`{}`))
}
func webNamedBuiltinRecoveryFixture(t *testing.T, service, name string, args json.RawMessage) (*Module, *executionAuthority, *executionBuiltinRuntime, LegacyExecuteInput) {
	t.Helper()
	m, port, runtime, in := builtinNamedExecutionFixture(t, service, name, args)
	oldClient := in.ClientID
	in.ClientID = "web-market"
	if err := m.store.update(func(d *database) error {
		install := d.Web.Installations[digest([]any{int64(1), oldClient, in.ToolID})]
		install.ClientID = in.ClientID
		d.Web.Installations[digest([]any{int64(1), in.ClientID, in.ToolID})] = install
		grant := legacyAccess(d).Grants[in.GrantID]
		grant.ClientID = in.ClientID
		legacyAccess(d).Grants[in.GrantID] = grant
		return nil
	}); err != nil {
		t.Fatal(err)
	}
	port.r.ClientID = in.ClientID
	port.r.ID = digest([]any{int64(1), in.ClientID, in.RequestKey})
	runtime.result = BuiltinResult{AwaitingConfirmation: true, DownstreamOperationID: "original", Continuation: json.RawMessage(`{"operation_id":"original","confirmation":"private"}`), ExecutionStatus: "previewed", SettlementStatus: "pending", Result: json.RawMessage(`{"structuredContent":{"confirmation_url":"https://core.example/confirm/original","confirmation":"private"}}`)}
	return m, port, runtime, in
}

func TestLegacyWebInspectNeverResumesConfirmedOperation(t *testing.T) {
	m, port, runtime, in := webBuiltinRecoveryFixture(t)
	r, err := m.LegacyExecute(context.Background(), "1", "1", in)
	if !errors.Is(err, ErrPending) {
		t.Fatal(err)
	}
	runtime.result.AwaitingConfirmation = false
	runtime.result.ResumeReady = true
	runtime.result.ExecutionStatus = "confirmed"
	for i := 0; i < 3; i++ {
		got, err := m.LegacyInspectExecution(context.Background(), "1", "1", r.ID)
		if !errors.Is(err, ErrPending) || got.State != "awaiting_confirmation" || got.Builtin.ResumeClaimed {
			t.Fatal(got, err)
		}
	}
	if runtime.invokes != 1 || runtime.resumes != 0 || port.reserves != 1 || port.finishes != 0 || port.releases != 0 {
		t.Fatal("read triggered financial or business mutation", runtime, port)
	}
	if _, err := m.LegacyInspectExecution(context.Background(), "2", "2", r.ID); !errors.Is(err, ErrMissing) {
		t.Fatal("cross-owner read", err)
	}
	runtime.resumeResult = BuiltinResult{DownstreamOperationID: "original", ExecutionStatus: "completed", SettlementStatus: "settled", Result: json.RawMessage(`{"content":[{"type":"text","text":"done"}],"structuredContent":{"ok":true}}`)}
	got, err := m.LegacyResumeWebExecution(context.Background(), "1", "1", r.ID, LegacyMCPRequestState(r), json.RawMessage(`{"approved":true}`))
	if err != nil || got.State != "done" || runtime.resumes != 1 || runtime.invokes != 1 || port.finishes != 1 {
		t.Fatal("explicit original resume failed", got, err, runtime)
	}
	if _, err = m.LegacyResumeWebExecution(context.Background(), "1", "1", r.ID, LegacyMCPRequestState(r), nil); err != nil || runtime.resumes != 1 {
		t.Fatal("replayed resume repeated generation", err)
	}
}

func TestLegacyWebLostPreviewAckLookupAndExplicitResume(t *testing.T) {
	m, port, runtime, in := webBuiltinRecoveryFixture(t)
	runtime.invokeErr = ErrPending
	r, err := m.LegacyExecute(context.Background(), "1", "1", in)
	if !errors.Is(err, ErrPending) || LegacyMCPRequestState(r) != "" {
		t.Fatal(r, err)
	}
	read, err := m.LegacyGetExecutionByRequestKey(context.Background(), "1", "1", "web-market", in.RequestKey)
	if !errors.Is(err, ErrPending) || len(LegacyMCPRequestState(read)) != 64 || runtime.invokes != 1 || runtime.resumes != 0 {
		t.Fatal(read, err)
	}
	var saved LegacyExecution
	_ = m.store.view(func(d database) error { saved = d.LegacyExecutions[r.ID]; return nil })
	if LegacyMCPRequestState(saved) != "" {
		t.Fatal("GET persisted a continuation")
	}
	runtime.result.ResumeReady = true
	runtime.result.AwaitingConfirmation = false
	runtime.resumeResult = BuiltinResult{DownstreamOperationID: "original", ExecutionStatus: "completed", SettlementStatus: "settled", Result: json.RawMessage(`{"content":[{"type":"text","text":"done"}],"structuredContent":{"ok":true}}`)}
	got, err := m.LegacyResumeWebExecution(context.Background(), "1", "1", r.ID, LegacyMCPRequestState(read), nil)
	if err != nil || got.State != "done" || runtime.invokes != 1 || runtime.resumes != 1 || port.reserves != 1 {
		t.Fatal("lost preview ACK restarted operation", got, err, runtime)
	}
}

func TestLegacyWebResumeUnknownNeverOffersOrRepeatsGeneration(t *testing.T) {
	m, port, runtime, in := webBuiltinRecoveryFixture(t)
	r, err := m.LegacyExecute(context.Background(), "1", "1", in)
	if !errors.Is(err, ErrPending) {
		t.Fatal(err)
	}
	runtime.result.ResumeReady = true
	runtime.result.AwaitingConfirmation = false
	runtime.resumeErr = ErrPending
	state := LegacyMCPRequestState(r)
	first, err := m.LegacyResumeWebExecution(context.Background(), "1", "1", r.ID, state, json.RawMessage(`{"approved":true}`))
	if !errors.Is(err, ErrPending) || !first.Builtin.ResumeClaimed || runtime.resumes != 1 {
		t.Fatal("original resume was not durably fenced", err)
	}
	for i := 0; i < 3; i++ {
		observed, err := m.LegacyInspectExecution(context.Background(), "1", "1", r.ID)
		if !errors.Is(err, ErrPending) {
			t.Fatal(err)
		}
		response, err := m.legacyWebExecution(observed, true)
		if err != nil || response.AwaitingConfirmation || response.Call["execution_status"] != "unknown" {
			t.Fatal("unknown generation offered continuation", response, err)
		}
		if _, err = m.LegacyResumeWebExecution(context.Background(), "1", "1", r.ID, state, nil); !errors.Is(err, ErrPending) {
			t.Fatal(err)
		}
	}
	if runtime.resumes != 1 || runtime.invokes != 1 || port.reserves != 1 || port.finishes != 0 || port.releases != 0 {
		t.Fatal("unknown generation repeated or released", runtime, port)
	}
	runtime.queryErr = ErrPending
	unknown, err := m.LegacyInspectExecution(context.Background(), "1", "1", r.ID)
	response, _ := m.legacyWebExecution(unknown, true)
	if !errors.Is(err, ErrPending) || response.AwaitingConfirmation || response.Call["execution_status"] != "unknown" {
		t.Fatal("lost query ACK treated as confirmation", err, response)
	}
}
