// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package toolmarket

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"errors"
	"net/http"
	"strings"
	"testing"
	"time"
)

type executionBuiltinRuntime struct {
	queryErr                  error
	resumeErr                 error
	resumeResult              BuiltinResult
	invokes, queries, resumes int
	result                    BuiltinResult
	invokeErr                 error
	last                      BuiltinInvocation
}

func (p *executionBuiltinRuntime) InvokeBuiltin(_ context.Context, _ string, in BuiltinInvocation) (BuiltinResult, error) {
	p.invokes++
	p.last = in
	r := p.result
	r.CallID = in.Context.CallID
	return r, p.invokeErr
}
func (p *executionBuiltinRuntime) QueryBuiltin(_ context.Context, _ string, in BuiltinInvocation) (BuiltinResult, error) {
	p.queries++
	p.last = in
	r := p.result
	r.CallID = in.Context.CallID
	return r, p.queryErr
}
func (p *executionBuiltinRuntime) ResumeBuiltin(_ context.Context, _ string, in BuiltinInvocation) (BuiltinResult, error) {
	p.resumes++
	p.last = in
	out := p.resumeResult
	out.CallID = in.Context.CallID
	return out, p.resumeErr
}
func builtinExecutionFixture(t *testing.T) (*Module, *executionAuthority, *executionBuiltinRuntime, LegacyExecuteInput) {
	t.Helper()
	return builtinNamedExecutionFixture(t, "wallet", "wallet.balance", json.RawMessage(`{}`))
}
func builtinNamedExecutionFixture(t *testing.T, service, name string, args json.RawMessage) (*Module, *executionAuthority, *executionBuiltinRuntime, LegacyExecuteInput) {
	t.Helper()
	definition, tool, e := compiledBuiltin(service, name)
	if e != nil {
		t.Fatal(e)
	}
	d := emptyDB()
	overlayBuiltinCatalog(&d)
	input := LegacyExecuteInput{ClientID: "web", RequestKey: "once", ToolID: tool.ToolID, VersionID: definition.VersionID, GrantID: "grant", Arguments: args}
	d.Web.Installations[digest([]any{int64(1), input.ClientID, input.ToolID})] = WebInstallation{UserID: 1, ClientID: input.ClientID, ToolID: input.ToolID, VersionID: input.VersionID}
	legacyAccess(&d).Grants["grant"] = LegacyConsentGrant{ID: "grant", UserID: 1, ClientID: input.ClientID, ToolID: input.ToolID, VersionID: input.VersionID, QuoteID: "quote", ExpiresAt: 1000}
	priceDigest, _ := LegacyQuotePricingDigest(tool.LegacyPricing())
	authority := &executionAuthority{executionCredits: executionCredits{r: LegacyCreditCall{ID: digest([]any{int64(1), input.ClientID, input.RequestKey}), Owner: Principal{User: "1", Account: "1"}, ClientID: input.ClientID, RequestKey: input.RequestKey, ToolID: input.ToolID, VersionID: input.VersionID, GrantID: "grant", QuoteID: "quote", InputDigest: digest(json.RawMessage(args)), PricingDigest: priceDigest, State: "running", DispatchID: "dispatch", DispatchClaimed: true, ResolveBy: 220}}}
	runtime := &executionBuiltinRuntime{result: BuiltinResult{Result: json.RawMessage(`{"content":[{"type":"text","text":"ok"}],"structuredContent":{"ok":true}}`), ExecutionStatus: "completed", SettlementStatus: "settled"}}
	m, e := New(&Store{data: d}, authority, Config{WebOnly: true, BuiltinRuntime: runtime})
	if e != nil {
		t.Fatal(e)
	}
	m.now = func() time.Time { return time.Unix(100, 0) }
	m.net.client = &http.Client{Transport: legacyProviderTransport(func(*http.Request) (*http.Response, error) {
		t.Fatal("builtin made remote request")
		return nil, ErrDenied
	})}
	return m, authority, runtime, input
}
func TestLegacyExecutionBuiltinUsesSameCoreReserveDispatchAndFinalization(t *testing.T) {
	m, port, runtime, input := builtinExecutionFixture(t)
	ctx := WithBuiltinPeerIP(context.Background(), "192.0.2.4")
	got, e := m.LegacyExecute(ctx, "1", "1", input)
	if e != nil || got.State != "done" || runtime.invokes != 1 || port.reserves != 1 || port.finishes != 1 || runtime.last.ActualPeerIP != "192.0.2.4" || runtime.last.Context.CallID != got.Core.ID || runtime.last.Context.DispatchID != "dispatch" {
		t.Fatalf("builtin skipped authority: %+v %v", got, e)
	}
	if got.AuthorityResultDigest == "" || len(got.RawResult) == 0 {
		t.Fatal("original result not retained")
	}
	finish := legacyFinishRequest(got)
	raw, e := base64.StdEncoding.DecodeString(finish.RawResult)
	if e != nil || string(raw) != string(got.RawResult) || finish.ResultDigest != got.AuthorityResultDigest {
		t.Fatal("raw proof binding changed")
	}
	if _, e = m.LegacyExecute(ctx, "1", "1", input); e != nil || runtime.invokes != 1 || runtime.queries != 0 || port.reserves != 1 {
		t.Fatal("completed builtin repeated", e)
	}
}
func TestLegacyExecutionBuiltinConfirmationQueriesThenResumesOriginalOperationOnce(t *testing.T) {
	m, port, runtime, input := builtinExecutionFixture(t)
	runtime.result = BuiltinResult{AwaitingConfirmation: true, Continuation: json.RawMessage(`{"operation_id":"original","confirmation":"private"}`), DownstreamOperationID: "original", Result: json.RawMessage(`{"content":[],"structuredContent":{"confirmation":"private"}}`), ExecutionStatus: "awaiting_confirmation", SettlementStatus: "pending"}
	got, e := m.LegacyExecute(context.Background(), "1", "1", input)
	if !errors.Is(e, ErrPending) || got.State != "awaiting_confirmation" || got.Result != nil || got.ResultDigest != "" || port.finishes != 0 {
		t.Fatalf("preview treated as final: %+v %v", got, e)
	}
	outer := LegacyMCPRequestState(got)
	if len(outer) != 64 || outer == "private" {
		t.Fatal("no opaque continuation binding")
	}
	runtime.result.ResumeReady = true
	runtime.result.AwaitingConfirmation = false
	runtime.result.ExecutionStatus = "confirmed"
	runtime.resumeResult = BuiltinResult{DownstreamOperationID: "original", Result: json.RawMessage(`{"content":[{"type":"text","text":"done"}],"structuredContent":{"ok":true}}`), ExecutionStatus: "completed", SettlementStatus: "settled"}
	got, e = m.LegacyExecuteMCP(context.Background(), "1", "1", input, outer, json.RawMessage(`{"approved":true}`))
	if e != nil || got.State != "done" || !got.Builtin.ResumeClaimed || runtime.invokes != 1 || runtime.queries != 1 || runtime.resumes != 1 || port.reserves != 1 || port.finishes != 1 {
		t.Fatalf("original resume failed %+v %v", got, e)
	}
	if string(runtime.last.Continuation) != `{"operation_id":"original","confirmation":"private"}` {
		t.Fatal("original continuation lost")
	}
	if _, e = m.LegacyExecuteMCP(context.Background(), "1", "1", input, outer, json.RawMessage(`{"approved":true}`)); e != nil || runtime.invokes != 1 || runtime.resumes != 1 || port.reserves != 1 {
		t.Fatal("continuation replay repeated effect", e)
	}
}
func TestLegacyExecutionBuiltinResumeUnknownOnlyQueriesNeverResumesAgain(t *testing.T) {
	m, port, runtime, input := builtinExecutionFixture(t)
	runtime.result = BuiltinResult{AwaitingConfirmation: true, Continuation: json.RawMessage(`{"operation_id":"original","confirmation":"private"}`), DownstreamOperationID: "original", ExecutionStatus: "awaiting_confirmation", SettlementStatus: "pending"}
	got, e := m.LegacyExecute(context.Background(), "1", "1", input)
	if !errors.Is(e, ErrPending) {
		t.Fatal(e)
	}
	state := LegacyMCPRequestState(got)
	// An MCP response is not authority: domain state is still awaiting, so no resume.
	if _, e = m.LegacyExecuteMCP(context.Background(), "1", "1", input, state, json.RawMessage(`{"approved":true}`)); !errors.Is(e, ErrPending) || runtime.resumes != 0 {
		t.Fatal("client response invented confirmation", e)
	}
	if _, e = m.LegacyExecuteMCP(context.Background(), "1", "1", input, strings.Repeat("f", 64), json.RawMessage(`{}`)); !errors.Is(e, ErrConflict) {
		t.Fatal("foreign request state accepted", e)
	}
	runtime.result.ResumeReady = true
	runtime.result.AwaitingConfirmation = false
	runtime.result.ExecutionStatus = "confirmed"
	runtime.resumeErr = ErrUpstream
	got, e = m.LegacyExecuteMCP(context.Background(), "1", "1", input, state, json.RawMessage(`{}`))
	if !errors.Is(e, ErrPending) || !got.Builtin.ResumeClaimed || !got.Builtin.Indeterminate {
		t.Fatal("ambiguous resume not retained", got, e)
	}
	m.now = func() time.Time { return time.Unix(500, 0) }
	for i := 0; i < 3; i++ {
		if _, e = m.LegacyExecuteMCP(context.Background(), "1", "1", input, state, json.RawMessage(`{}`)); !errors.Is(e, ErrPending) {
			t.Fatal(e)
		}
	}
	if runtime.invokes != 1 || runtime.resumes != 1 || port.reserves != 1 || port.finishes != 0 || port.releases != 0 {
		t.Fatalf("UNKNOWN repeated effect %+v %+v", runtime, port.executionCredits)
	}
}
func TestLegacyExecutionBuiltinUnknownNeverInvokesAgain(t *testing.T) {
	m, port, runtime, input := builtinExecutionFixture(t)
	runtime.invokeErr = ErrUpstream
	got, e := m.LegacyExecute(context.Background(), "1", "1", input)
	if !errors.Is(e, ErrPending) || got.State != "unknown" {
		t.Fatal(got, e)
	}
	runtime.result = BuiltinResult{ExecutionStatus: "unknown", SettlementStatus: "pending"}
	runtime.invokeErr = nil
	m.now = func() time.Time { return time.Unix(500, 0) }
	for i := 0; i < 2; i++ {
		got, e = m.LegacyExecute(context.Background(), "1", "1", input)
		if !errors.Is(e, ErrPending) {
			t.Fatal(e)
		}
	}
	if runtime.invokes != 1 || runtime.queries != 2 || runtime.resumes != 0 || port.reserves != 1 || port.finishes != 0 || port.releases != 0 {
		t.Fatalf("unknown repeated business effects %+v %+v", runtime, port.executionCredits)
	}
}
func TestLegacyExecutionBuiltinCompiledBindingAndRemoteOnlyBoundary(t *testing.T) {
	m, port, runtime, input := builtinExecutionFixture(t)
	if _, e := m.LegacyExecuteRemote(context.Background(), "1", "1", input); !errors.Is(e, ErrUnavailable) || port.reserves != 0 || runtime.invokes != 0 {
		t.Fatal("remote API selected builtin", e)
	}
	m.builtinRuntime = (*executionBuiltinRuntime)(nil)
	if _, e := m.LegacyExecute(context.Background(), "1", "1", input); !errors.Is(e, ErrUnavailable) || port.reserves != 0 {
		t.Fatal("typed nil runtime accepted", e)
	}
}

func TestLegacyExecutionBuiltinStaleQueryCannotClearDurableResumeFence(t *testing.T) {
	m, port, runtime, input := builtinExecutionFixture(t)
	runtime.result = BuiltinResult{AwaitingConfirmation: true, Continuation: json.RawMessage(`{"operation_id":"original"}`), DownstreamOperationID: "original", ExecutionStatus: "awaiting_confirmation", SettlementStatus: "pending"}
	stale, err := m.LegacyExecute(context.Background(), "1", "1", input)
	if !errors.Is(err, ErrPending) {
		t.Fatal(err)
	}
	if err = m.store.update(func(d *database) error {
		r := d.LegacyExecutions[stale.ID]
		r.Builtin.ResumeClaimed = true
		r.Builtin.Indeterminate = true
		r.State = "unknown"
		d.LegacyExecutions[r.ID] = r
		return nil
	}); err != nil {
		t.Fatal(err)
	}
	// A query started before the winning claim returns a confirmed preview later.
	runtime.result.AwaitingConfirmation = false
	runtime.result.ResumeReady = true
	runtime.result.ExecutionStatus = "confirmed"
	if _, err = m.legacyQueryBuiltin(context.Background(), port, "1", stale); !errors.Is(err, ErrPending) {
		t.Fatal(err)
	}
	if err = m.store.view(func(d database) error {
		r := d.LegacyExecutions[stale.ID]
		if !r.Builtin.ResumeClaimed || !r.Builtin.Indeterminate || r.State != "unknown" {
			t.Fatal("stale query removed durable fence", r)
		}
		return nil
	}); err != nil {
		t.Fatal(err)
	}
	if runtime.resumes != 0 || runtime.invokes != 1 {
		t.Fatal("stale query re-executed downstream")
	}
}
