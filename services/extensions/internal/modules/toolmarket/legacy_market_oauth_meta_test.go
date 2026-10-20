// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package toolmarket

import (
	"context"
	"encoding/json"
	"strings"
	"testing"
)

type marketMetaFixture struct {
	*marketAuthFixture
	view   LegacyMarketMetaView
	owner  LegacyMarketMetaOwnerView
	grant  CoreLegacyConsentReceipt
	budget CoreLegacyBudgetReceipt
	usage  LegacyMarketMetaUsage
	calls  LegacyMarketMetaCalls
}

func (f *marketMetaFixture) ReadLegacyMarketMeta(context.Context, string) (LegacyMarketMetaView, error) {
	return f.view, f.fail
}
func (f *marketMetaFixture) ReadLegacyMarketMetaOwner(context.Context, string, LegacyMarketMetaTarget) (LegacyMarketMetaOwnerView, error) {
	return f.owner, f.fail
}
func (f *marketMetaFixture) SetLegacyMarketMetaOwner(_ context.Context, _ string, in LegacyMarketMetaSet) (LegacyMarketMetaOwnerView, error) {
	out := f.owner
	out.OperationID = in.OperationID
	return out, f.fail
}
func (f *marketMetaFixture) AuthorizeLegacyMarketMeta(context.Context, string, LegacyMarketMetaAuthorize) (CoreLegacyConsentReceipt, error) {
	return f.grant, f.fail
}
func (f *marketMetaFixture) TightenLegacyMarketClientBudget(_ context.Context, _ string, in LegacyMarketMetaClientBudget) (CoreLegacyBudgetReceipt, error) {
	out := f.budget
	out.OperationID = in.OperationID
	return out, f.fail
}
func (f *marketMetaFixture) TightenLegacyMarketToolBudget(context.Context, string, LegacyMarketMetaToolBudget) (CoreLegacyConsentReceipt, error) {
	return f.grant, f.fail
}
func (f *marketMetaFixture) ReadLegacyMarketUsage(context.Context, string, LegacyMarketMetaPage) (LegacyMarketMetaUsage, error) {
	return f.usage, f.fail
}
func (f *marketMetaFixture) ReadLegacyMarketCalls(context.Context, string, LegacyMarketMetaPage) (LegacyMarketMetaCalls, error) {
	return f.calls, f.fail
}
func metaConsumerFixture(t *testing.T) (*Module, *marketMetaFixture, LegacyMarketPrincipal) {
	m, base := legacyCoreModule(t)
	a := &marketAuthFixture{legacyCoreFixture: base}
	p := LegacyMarketPrincipal{UserID: "1", AccountID: "1", ClientID: "Client", CredentialKind: "personal", CredentialID: "token", CanInvoke: true, CanManage: true, ExpiresAt: 200}
	owner := Principal{User: "1", Account: "1"}
	f := &marketMetaFixture{marketAuthFixture: a}
	f.view = LegacyMarketMetaView{Owner: owner, ClientID: p.ClientID, Delegation: LegacyMarketDelegation{Enabled: true, MaxTotalQuota: 100, ExpiresAt: 200, UpdatedAt: 99}}
	f.owner = LegacyMarketMetaOwnerView{Owner: owner, Kind: "personal", ID: "token", Delegation: f.view.Delegation}
	f.grant = CoreLegacyConsentReceipt{CoreLegacyConsentRequest: CoreLegacyConsentRequest{CoreLegacyOperation: CoreLegacyOperation{Owner: owner, OperationID: "original"}, ClientID: "Client", ToolID: "tool", VersionID: "version", MaxPriceQuota: 10, MaxTotalQuota: 100, MaxCalls: 10, ExpiresAt: 200}, ID: "grant", QuoteID: "quote", FrozenPriceQuota: 10, RulesHash: strings.Repeat("a", 64), CreatedAt: 90, ReservedQuota: 10, SpentQuota: 20, ReservedCalls: 2, SuccessfulCalls: 1}
	f.budget = CoreLegacyBudgetReceipt{CoreLegacyBudgetRequest: CoreLegacyBudgetRequest{CoreLegacyOperation: CoreLegacyOperation{Owner: owner}, Scope: "client", ScopeID: "Client", LimitQuota: 100}, ReservedQuota: 10, SpentQuota: 20}
	f.usage = LegacyMarketMetaUsage{Owner: owner, ClientID: "Client", ClientBudget: &f.budget, Grants: []CoreLegacyConsentReceipt{f.grant}, Limit: 20}
	f.calls = LegacyMarketMetaCalls{Owner: owner, ClientID: "Client", Calls: []LegacyMarketMetaCall{{CallID: "call", ServiceID: "service", ToolID: "tool", VersionID: "version", PriceQuota: 10, ExecutionStatus: "succeeded", SettlementStatus: "settled", CreatedAt: 90, StartedAt: 91, FinishedAt: 92}}, Limit: 20}
	m.authority = f
	return m, f, p
}
func TestLegacyMarketMetaConsumesCoreCountersAndRejectsAnotherClient(t *testing.T) {
	m, f, p := metaConsumerFixture(t)
	in := legacyMCPInput{Action: "usage", Limit: 20}
	out, e := m.legacyMCPManagement(t.Context(), "raw", p, in)
	if e != nil {
		t.Fatal(e)
	}
	encoded, _ := json.Marshal(out)
	if !strings.Contains(string(encoded), `"reserved_quota":10`) || !strings.Contains(string(encoded), `"spent_quota":20`) || strings.Contains(string(encoded), "operation_id") {
		t.Fatal(string(encoded))
	}
	f.usage.ClientID = "other"
	if _, e = m.legacyMCPManagement(t.Context(), "raw", p, in); e != ErrDenied {
		t.Fatal("cross-client counters", e)
	}
	f.usage.ClientID = p.ClientID
	f.usage.ClientBudget.ReservedQuota = 100
	if _, e = m.legacyMCPManagement(t.Context(), "raw", p, in); e != ErrDenied {
		t.Fatal("invalid counters", e)
	}
}
func TestLegacyMarketMetaOwnerAndGrantReadbackCannotForgeOwner(t *testing.T) {
	m, f, p := metaConsumerFixture(t)
	request := LegacyMarketMetaSet{OperationID: "owner-set", Kind: "personal", ID: "token", Enabled: true, MaxTotalQuota: 100, ExpiresAt: 200}
	if _, e := m.LegacySetMetaDelegation(t.Context(), "1", "1", request); e != nil {
		t.Fatal(e)
	}
	f.owner.Owner.Account = "2"
	if _, e := m.LegacySetMetaDelegation(t.Context(), "1", "1", request); e != ErrDenied {
		t.Fatal(e)
	}
	input := legacyMCPInput{Action: "authorize", ToolID: "tool", VersionID: "version", MaxPriceQuota: 10, MaxTotalQuota: 100, MaxCalls: 10, ExpiresAt: 200}
	if _, e := m.legacyMCPManagement(t.Context(), "raw", p, input); e != nil {
		t.Fatal(e)
	}
	f.grant.ClientID = "other"
	if _, e := m.legacyMCPManagement(t.Context(), "raw", p, input); e != ErrDenied {
		t.Fatal(e)
	}
}
func TestLegacyMarketMetaCallsUseRealTimestampsAndNoPrivatePayload(t *testing.T) {
	m, f, p := metaConsumerFixture(t)
	input := legacyMCPInput{Action: "calls", Limit: 20}
	out, e := m.legacyMCPManagement(t.Context(), "raw", p, input)
	if e != nil {
		t.Fatal(e)
	}
	raw, _ := json.Marshal(out)
	if strings.Contains(string(raw), "owner") || strings.Contains(string(raw), "request_key") || !strings.Contains(string(raw), `"started_at":91`) {
		t.Fatal(string(raw))
	}
	f.calls.Calls[0].StartedAt = 0
	if _, e = m.legacyMCPManagement(t.Context(), "raw", p, input); e != ErrDenied {
		t.Fatal("unknown start time invented", e)
	}
	for _, bad := range []string{`{"enabled":false,"max_total_quota":0,"expires_at":0}`, `{"enabled":false,"max_total_quota":0,"expires_at":0,"updated_at":null}`, `{"enabled":false,"enabled":true,"max_total_quota":0,"expires_at":0,"updated_at":0}`} {
		var v LegacyMarketDelegation
		if json.Unmarshal([]byte(bad), &v) == nil {
			t.Fatal("malformed policy", bad)
		}
	}
}
func TestLegacyMCPLocalProfileRequiresSameNumericLoopbackAndHighPorts(t *testing.T) {
	valid := LegacyMarketMCPOptions{LocalHTTP: true, Issuer: "http://127.0.0.1:9001/api/oauth2/market", Resource: "http://127.0.0.1:9002/mcp/market", ResourceMetadataURL: "http://127.0.0.1:9001/.well-known/oauth-protected-resource/mcp/market"}
	if !legacyMCPOptionsValid(valid) {
		t.Fatal("valid explicit local profile denied")
	}
	for _, uri := range []string{"http://localhost:9001/api/oauth2/market", "http://127.0.0.2:9001/api/oauth2/market", "http://127.0.0.1:80/api/oauth2/market", "http://192.168.1.1:9001/api/oauth2/market"} {
		bad := valid
		bad.Issuer = uri
		if legacyMCPOptionsValid(bad) {
			t.Fatal(uri)
		}
	}
	valid.LocalHTTP = false
	if legacyMCPOptionsValid(valid) {
		t.Fatal("HTTP implicitly enabled")
	}
}
