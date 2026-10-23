// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package assistant_market

import (
	"context"
	"encoding/json"
	"errors"
	"strings"
	"testing"

	"github.com/TokenNotIncluded/lmm-api/services/extensions/internal/modules/assistant"
	"github.com/TokenNotIncluded/lmm-api/services/extensions/internal/modules/assistant/access"
	"github.com/TokenNotIncluded/lmm-api/services/extensions/internal/modules/toolmarket"
)

type marketFixture struct {
	detail                         toolmarket.WebDetail
	quote                          toolmarket.LegacyCreditQuote
	quotes                         int
	quoteErr                       error
	record                         toolmarket.LegacyExecution
	executeErr, inspectErr         error
	executes, inspects, catalogues int
	input                          toolmarket.LegacyExecuteInput
	user                           int64
	authErr                        error
	client, key                    string
}

func (f *marketFixture) WebAuthorize(context.Context, string, string, string) (int64, error) {
	return f.user, f.authErr
}
func (f *marketFixture) PublicWebCatalogue(context.Context, string, string, toolmarket.WebQuery) ([]toolmarket.WebSummary, error) {
	return []toolmarket.WebSummary{{ID: f.detail.Service.ID, VersionID: f.detail.Version.ID}}, nil
}
func (f *marketFixture) PublicWebDetail(context.Context, string, string, string) (toolmarket.WebDetail, error) {
	f.catalogues++
	return f.detail, nil
}
func (f *marketFixture) LegacyReadToolQuote(context.Context, string, string, string, string, string) (toolmarket.LegacyCreditQuote, error) {
	f.quotes++
	return f.quote, f.quoteErr
}
func (f *marketFixture) LegacyExecuteRemote(_ context.Context, _ string, _ string, input toolmarket.LegacyExecuteInput) (toolmarket.LegacyExecution, error) {
	f.executes++
	f.input = input
	return f.record, f.executeErr
}
func (f *marketFixture) LegacyGetExecutionByRequestKey(_ context.Context, _ string, _ string, client, key string) (toolmarket.LegacyExecution, error) {
	f.inspects++
	f.client = client
	f.key = key
	return f.record, f.inspectErr
}
func setup(t *testing.T) (*Executor, *marketFixture, assistant.AgentInvocation) {
	t.Helper()
	e, err := New(&toolmarket.Module{})
	if err != nil {
		t.Fatal(err)
	}
	tool := toolmarket.WebTool{ToolID: "remote-tool", VersionID: "version-one", Name: "echo", InputSchema: `{"type":"object","properties":{"message":{"type":"string"}},"required":["message"],"additionalProperties":false}`}
	price, err := toolmarket.LegacyQuotePricingDigest(tool.LegacyPricing())
	if err != nil {
		t.Fatal(err)
	}
	b := e.Binding()
	raw := json.RawMessage(`{"service_id":"service-one","tool_id":"remote-tool","version_id":"version-one","grant_id":"grant-one","arguments":{"message":"hello"}}`)
	in := assistant.AgentInvocation{Credential: "private-browser-credential", Actor: access.Principal{UserID: 7, Account: access.Account{Kind: "personal", ID: 7}}, Call: assistant.AgentToolCall{OperationID: "core-tool-operation-one", Scope: assistant.AgentToolScope{Name: b.Name, Version: b.Version, SchemaSHA256: e.definition.SchemaSHA256, PriceVersion: b.PriceVersion, PriceSHA256: b.PriceSHA256, Domain: "extension", ResourceKind: "toolmarket", ResourceID: tool.ToolID, ResourceVersion: tool.VersionID, ResourceSchemaSHA256: strings.Repeat("d", 64), ResourcePriceSHA256: price, GrantID: "grant-one"}, Arguments: raw, ArgumentsSHA256: digest(raw)}}
	owner := toolmarket.Principal{User: "7", Account: "7"}
	record := toolmarket.LegacyExecution{ID: "module-operation", Owner: owner, Input: toolmarket.LegacyExecuteInput{ClientID: ClientID, RequestKey: in.Call.OperationID, ToolID: tool.ToolID, VersionID: tool.VersionID, GrantID: "grant-one", Arguments: json.RawMessage(`{"message":"hello"}`)}, MeteringBinding: toolmarket.LegacyMeteringBinding{ServiceID: "service-one", VersionID: tool.VersionID, ToolID: tool.ToolID, RemoteDigest: in.Call.Scope.ResourceSchemaSHA256}, SettlementPricingDigest: price, State: "done", Success: true, Result: json.RawMessage(`{"content":[{"type":"text","text":"safe answer"}]}`), Core: toolmarket.LegacyCreditCall{ID: "original-core-call", Owner: owner, ClientID: ClientID, RequestKey: in.Call.OperationID, ToolID: tool.ToolID, VersionID: tool.VersionID, GrantID: "grant-one", PricingDigest: price, State: "settled"}}
	record.ResultDigest = digest(record.Result)
	f := &marketFixture{quote: toolmarket.LegacyCreditQuote{ID: "frozen-quote", ServiceID: "service-one", ToolID: tool.ToolID, VersionID: tool.VersionID, RemoteDigest: in.Call.Scope.ResourceSchemaSHA256, PricingDigest: price}, user: 7, record: record, detail: toolmarket.WebDetail{Service: toolmarket.WebServiceRecord{ID: "service-one", LiveVersionID: tool.VersionID, OwnerID: 88}, Version: toolmarket.WebVersion{ID: tool.VersionID, ServiceID: "service-one", ExecutionType: "remote", Endpoint: "https://private.invalid"}, Tools: []toolmarket.WebTool{tool}, Validated: true}}
	e.market = f
	return e, f, in
}
func TestBindingUsesExactFixedMainSchemaAndFreeOuterPrice(t *testing.T) {
	e, _, _ := setup(t)
	b := e.Binding()
	if b.Name != "call_market_tool" || b.Effect != "server_guarded" || b.Access != "l1" || b.Version != assistant.FixedMainToolVersion || b.PriceSHA256 != assistant.FreeAgentPriceSHA256() || digest(b.Parameters) != e.definition.SchemaSHA256 {
		t.Fatalf("wrong binding: %+v", b)
	}
	if _, err := assistant.NewAgentRegistry([]assistant.AgentToolBinding{b}); err != nil {
		t.Fatal(err)
	}
	b.Parameters[0] = 'x'
	if !json.Valid(e.Binding().Parameters) {
		t.Fatal("binding mutated installed schema")
	}
}
func TestExecuteUsesOriginalOperationAndProjectsOnlyPublicResult(t *testing.T) {
	e, f, in := setup(t)
	f.record.RawResult = []byte("PRIVATE_RAW")
	f.record.ProviderOwnerID = 999
	got, err := e.Execute(context.Background(), in)
	if err != nil {
		t.Fatal(err)
	}
	if got.State != assistant.AgentToolSucceeded || got.DomainReceiptID != "original-core-call" || f.executes != 1 || f.input.ClientID != ClientID || f.input.RequestKey != in.Call.OperationID {
		t.Fatalf("unexpected outcome: %+v %+v", got, f.input)
	}
	for _, private := range []string{"PRIVATE_RAW", "provider_owner", "\"owner\"", "private.invalid", in.Credential} {
		if strings.Contains(string(got.ResultJSON), private) {
			t.Fatal("private field leaked", private)
		}
	}
	if digest(got.ResultJSON) != got.ResultSHA256 {
		t.Fatal("result digest")
	}
}
func TestExecuteRejectsMismatchedAuthorityBeforeDispatch(t *testing.T) {
	cases := map[string]func(*marketFixture, *assistant.AgentInvocation){
		"tool":    func(_ *marketFixture, i *assistant.AgentInvocation) { i.Call.Scope.ResourceID = "other-tool" },
		"version": func(_ *marketFixture, i *assistant.AgentInvocation) { i.Call.Scope.ResourceVersion = "other-version" },
		"grant":   func(_ *marketFixture, i *assistant.AgentInvocation) { i.Call.Scope.GrantID = "other-grant" },
		"schema": func(_ *marketFixture, i *assistant.AgentInvocation) {
			i.Call.Scope.ResourceSchemaSHA256 = strings.Repeat("a", 64)
		},
		"price": func(_ *marketFixture, i *assistant.AgentInvocation) {
			i.Call.Scope.ResourcePriceSHA256 = strings.Repeat("a", 64)
		},
		"service":           func(f *marketFixture, _ *assistant.AgentInvocation) { f.detail.Version.ServiceID = "another-service" },
		"catalogue_version": func(f *marketFixture, _ *assistant.AgentInvocation) { f.detail.Version.ID = "another-version" },
		"tool_version":      func(f *marketFixture, _ *assistant.AgentInvocation) { f.detail.Tools[0].VersionID = "another-version" },
		"duplicate_tool": func(f *marketFixture, _ *assistant.AgentInvocation) {
			f.detail.Tools = append(f.detail.Tools, f.detail.Tools[0])
		},
		"owner":      func(f *marketFixture, _ *assistant.AgentInvocation) { f.user = 88 },
		"permission": func(f *marketFixture, _ *assistant.AgentInvocation) { f.authErr = toolmarket.ErrDenied },
		"outer_price": func(_ *marketFixture, i *assistant.AgentInvocation) {
			i.Call.Scope.PriceSHA256 = strings.Repeat("a", 64)
		},
		"unknown_field": func(_ *marketFixture, i *assistant.AgentInvocation) {
			i.Call.Arguments = json.RawMessage(strings.Replace(string(i.Call.Arguments), `"service_id":`, `"payer":7,"service_id":`, 1))
			i.Call.ArgumentsSHA256 = digest(i.Call.Arguments)
		},
		"duplicate_field": func(_ *marketFixture, i *assistant.AgentInvocation) {
			i.Call.Arguments = json.RawMessage(strings.Replace(string(i.Call.Arguments), `"service_id":`, `"service_id":"evil","service_id":`, 1))
			i.Call.ArgumentsSHA256 = digest(i.Call.Arguments)
		},
		"nested_schema": func(_ *marketFixture, i *assistant.AgentInvocation) {
			i.Call.Arguments = json.RawMessage(strings.Replace(string(i.Call.Arguments), `"message":"hello"`, `"message":8`, 1))
			i.Call.ArgumentsSHA256 = digest(i.Call.Arguments)
		},
	}
	for name, mutate := range cases {
		t.Run(name, func(t *testing.T) {
			e, f, in := setup(t)
			mutate(f, &in)
			if _, err := e.Execute(context.Background(), in); err == nil {
				t.Fatal("accepted mismatch")
			}
			if f.executes != 0 {
				t.Fatal("dispatched rejected claim")
			}
		})
	}
}
func TestInspectMissingAndPendingNeverDispatch(t *testing.T) {
	for _, state := range []string{"missing", "prepared", "reserved", "result", "unknown"} {
		t.Run(state, func(t *testing.T) {
			e, f, in := setup(t)
			f.detail = toolmarket.WebDetail{}
			want := assistant.AgentToolPending
			if state == "missing" {
				f.record = toolmarket.LegacyExecution{}
				f.inspectErr = toolmarket.ErrMissing
				want = assistant.AgentToolUnknown
			} else {
				f.record.State = state
				f.inspectErr = toolmarket.ErrPending
				if state == "unknown" {
					want = assistant.AgentToolUnknown
				}
			}
			got, err := e.Inspect(context.Background(), in)
			if err != nil {
				t.Fatal(err)
			}
			if got.State != want || got.DomainReceiptID != "" || f.executes != 0 || f.inspects != 1 || f.catalogues != 0 || f.quotes != 0 || f.client != ClientID || f.key != in.Call.OperationID {
				t.Fatalf("unexpected recovery %+v calls=%d/%d", got, f.executes, f.inspects)
			}
			if strings.Contains(string(got.ResultJSON), "safe answer") {
				t.Fatal("unsettled result exposed")
			}
		})
	}
}
func TestInspectOriginalBindingAndTerminalState(t *testing.T) {
	e, f, in := setup(t)
	f.detail = toolmarket.WebDetail{}
	got, err := e.Inspect(context.Background(), in)
	if err != nil || got.State != assistant.AgentToolSucceeded || f.catalogues != 0 || f.executes != 0 {
		t.Fatalf("original recovery: %+v %v", got, err)
	}
	f.record.Input.GrantID = "replacement-grant"
	if _, err = e.Inspect(context.Background(), in); !errors.Is(err, access.ErrConflict) {
		t.Fatal("replacement grant accepted", err)
	}
}
func TestReleasedFailedExpiredAndUnsafeResult(t *testing.T) {
	for _, scenario := range []string{"released", "business_failure", "expired", "tampered", "secret"} {
		t.Run(scenario, func(t *testing.T) {
			e, f, in := setup(t)
			want := assistant.AgentToolFailed
			switch scenario {
			case "released":
				f.record.Core.State = "released"
			case "business_failure":
				f.record.Success = false
			case "expired":
				f.record.Result = nil
				want = assistant.AgentToolUnknown
			case "tampered":
				f.record.Result = json.RawMessage(`{"content":"tampered"}`)
				want = assistant.AgentToolUnknown
			case "secret":
				f.record.Result = json.RawMessage(`{"content":"private-browser-credential"}`)
				f.record.ResultDigest = digest(f.record.Result)
				want = assistant.AgentToolSucceeded
			}
			got, err := e.Inspect(context.Background(), in)
			if err != nil || got.State != want {
				t.Fatalf("%+v %v", got, err)
			}
			if strings.Contains(string(got.ResultJSON), in.Credential) {
				t.Fatal("credential exposed")
			}
		})
	}
}

func readInvocation(t *testing.T, e *Executor, in assistant.AgentInvocation, name string, raw string) (assistant.AgentToolExecutor, assistant.AgentInvocation) {
	t.Helper()
	bindings, err := e.Bindings()
	if err != nil {
		t.Fatal(err)
	}
	if len(bindings) != 3 {
		t.Fatal("unexpected advertised tools", len(bindings))
	}
	for _, b := range bindings {
		if b.Name == name {
			in.Call.Scope = assistant.AgentToolScope{Name: b.Name, Version: b.Version, SchemaSHA256: digest(b.Parameters), PriceVersion: b.PriceVersion, PriceSHA256: b.PriceSHA256, Domain: "extension"}
			in.Call.Arguments = json.RawMessage(raw)
			in.Call.ArgumentsSHA256 = digest(in.Call.Arguments)
			return b.Executor, in
		}
	}
	t.Fatal("missing read binding", name)
	return nil, in
}
func TestPublicCatalogueReadsRealVersionsAndHidesPrivateMetadata(t *testing.T) {
	for _, name := range []string{"get_tool_market_services", "get_tool_market_service"} {
		t.Run(name, func(t *testing.T) {
			e, f, in := setup(t)
			id := "11111111-1111-4111-8111-111111111111"
			f.detail.Service.ID = id
			f.detail.Version.ServiceID = id
			args := `{}`
			if name == "get_tool_market_service" {
				args = `{"service_id":"` + id + `"}`
			}
			executor, in := readInvocation(t, e, in, name, args)
			got, err := executor.Execute(context.Background(), in)
			if err != nil || got.State != assistant.AgentToolSucceeded {
				t.Fatalf("read %+v %v", got, err)
			}
			for _, secret := range []string{"owner_id", "private.invalid", "review_note", in.Credential} {
				if strings.Contains(string(got.ResultJSON), secret) {
					t.Fatal("private catalogue metadata leaked", secret)
				}
			}
			if name == "get_tool_market_service" {
				var result struct {
					Service struct {
						Tools []struct {
							Schema string `json:"input_schema_json"`
							SHA    string `json:"input_schema_sha256"`
						}
					}
				}
				if json.Unmarshal(got.ResultJSON, &result) != nil || len(result.Service.Tools) != 1 {
					t.Fatal("missing tools")
				}
				tool := result.Service.Tools[0]
				if tool.Schema != f.detail.Tools[0].InputSchema || tool.SHA != digest([]byte(tool.Schema)) {
					t.Fatal("schema bytes changed")
				}
			}
			if f.executes != 0 || f.inspects != 0 {
				t.Fatal("catalogue read executed a tool")
			}
			if _, err = executor.Inspect(context.Background(), in); err != nil {
				t.Fatal(err)
			}
		})
	}
}
func TestCatalogueReadRejectsSnapshotMismatchAndCannotAdvertiseConnections(t *testing.T) {
	e, f, in := setup(t)
	bindings, err := e.Bindings()
	if err != nil {
		t.Fatal(err)
	}
	for _, b := range bindings {
		if b.Name == "connect_market_tool" || b.Name == "get_connected_market_tools" {
			t.Fatal("unimplemented capability advertised")
		}
	}
	executor, in := readInvocation(t, e, in, "get_tool_market_services", `{}`)
	f.detail.Service.LiveVersionID = "replacement-version"
	if _, err = executor.Execute(context.Background(), in); !errors.Is(err, access.ErrConflict) {
		t.Fatal("mixed snapshot accepted", err)
	}
}
func TestSchemaChangedByRedactionIsExplicitlyOmitted(t *testing.T) {
	e, f, in := setup(t)
	id := "11111111-1111-4111-8111-111111111111"
	f.detail.Service.ID = id
	f.detail.Version.ServiceID = id
	f.detail.Tools[0].InputSchema = `{"type":"object","properties":{"token":{"type":"string"}},"additionalProperties":false}`
	executor, in := readInvocation(t, e, in, "get_tool_market_service", `{"service_id":"`+id+`"}`)
	got, err := executor.Execute(context.Background(), in)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(got.ResultJSON), `"schema_omitted":true`) || strings.Contains(string(got.ResultJSON), "input_schema_json") {
		t.Fatalf("altered schema presented as exact: %s", got.ResultJSON)
	}
}

func TestInputSchemaDigestIsNotRemoteAuthorityDigest(t *testing.T) {
	e, f, in := setup(t)
	f.detail.Tools[0].InputSchema = " \n" + f.detail.Tools[0].InputSchema + "\n"
	in.Call.Scope.ResourceSchemaSHA256 = digest([]byte(f.detail.Tools[0].InputSchema))
	if _, err := e.Execute(context.Background(), in); !errors.Is(err, access.ErrConflict) || f.executes != 0 {
		t.Fatal("standalone input schema digest accepted as remote authority", err)
	}
	in.Call.Scope.ResourceSchemaSHA256 = f.quote.RemoteDigest
	if _, err := e.Execute(context.Background(), in); err != nil || f.executes != 1 {
		t.Fatal("frozen full-tool digest rejected", err)
	}
}

// Core remote scopes bind the entire frozen remote tool, not InputSchema alone.
func TestExecutionBindsFullRemoteQuoteDigestNotInputSchema(t *testing.T) {
	e, f, in := setup(t)
	remoteDigest := strings.Repeat("e", 64)
	f.quote.RemoteDigest = remoteDigest
	in.Call.Scope.ResourceSchemaSHA256 = remoteDigest
	f.record.MeteringBinding.RemoteDigest = remoteDigest
	if _, err := e.Execute(context.Background(), in); err != nil {
		t.Fatalf("full remote quote digest rejected: %v", err)
	}
}

func TestFrozenQuoteMismatchCannotDispatch(t *testing.T) {
	for _, which := range []string{"service", "version", "tool", "schema", "price", "missing_id", "unavailable"} {
		t.Run(which, func(t *testing.T) {
			e, f, in := setup(t)
			switch which {
			case "service":
				f.quote.ServiceID = "other"
			case "version":
				f.quote.VersionID = "other"
			case "tool":
				f.quote.ToolID = "other"
			case "schema":
				f.quote.RemoteDigest = strings.Repeat("e", 64)
			case "price":
				f.quote.PricingDigest = strings.Repeat("e", 64)
			case "missing_id":
				f.quote.ID = ""
			case "unavailable":
				f.quoteErr = toolmarket.ErrUnavailable
			}
			if _, err := e.Execute(context.Background(), in); err == nil || f.executes != 0 {
				t.Fatal("invalid quote accepted", err)
			}
		})
	}
}
func TestInspectRejectsChangedOriginalRemoteDigestWithoutQuoteLookup(t *testing.T) {
	e, f, in := setup(t)
	f.record.MeteringBinding.RemoteDigest = strings.Repeat("e", 64)
	if _, err := e.Inspect(context.Background(), in); !errors.Is(err, access.ErrConflict) {
		t.Fatal("original full-tool digest mismatch accepted", err)
	}
	if f.quotes != 0 || f.catalogues != 0 || f.executes != 0 {
		t.Fatal("recovery read live quote/catalogue or dispatched")
	}
}
func TestReadExposesDistinctSchemaBytesAndResourceQuoteDigests(t *testing.T) {
	e, f, in := setup(t)
	id := "11111111-1111-4111-8111-111111111111"
	f.detail.Service.ID = id
	f.detail.Version.ServiceID = id
	f.quote.ServiceID = id
	executor, in := readInvocation(t, e, in, "get_tool_market_service", `{"service_id":"`+id+`"}`)
	got, err := executor.Execute(context.Background(), in)
	if err != nil {
		t.Fatal(err)
	}
	var out struct {
		Service struct {
			Tools []struct {
				Input    string `json:"input_schema_sha256"`
				Resource string `json:"resource_schema_sha256"`
				Price    string `json:"resource_price_sha256"`
			}
		}
	}
	if json.Unmarshal(got.ResultJSON, &out) != nil || len(out.Service.Tools) != 1 {
		t.Fatal("missing tool metadata")
	}
	row := out.Service.Tools[0]
	if row.Input != digest([]byte(f.detail.Tools[0].InputSchema)) || row.Resource != f.quote.RemoteDigest || row.Price != f.quote.PricingDigest || row.Input == row.Resource {
		t.Fatal("schema/resource digests confused", row)
	}
}
