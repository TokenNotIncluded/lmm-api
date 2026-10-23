package toolmarket

import (
	"context"
	"encoding/json"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/wallet"
	"strings"
	"testing"
)

type builtinGateStub struct {
	calls int
	alter bool
}

func (g *builtinGateStub) AuthorizeBuiltin(_ context.Context, _ string, in BuiltinAuthorizationRequest) (BuiltinAuthorization, error) {
	g.calls++
	r := BuiltinAuthorization{Owner: Principal{"7", "7"}, CallID: in.Context.CallID, DispatchID: in.Context.DispatchID, ToolID: in.Context.ToolID, VersionID: in.Context.VersionID, InputDigest: in.Context.InputDigest, InputSchemaDigest: in.InputSchemaDigest}
	if g.alter {
		r.DispatchID = "another-dispatch"
	}
	return r, nil
}
func builtinTestInvocation(t *testing.T, name string, args string) BuiltinInvocation {
	t.Helper()
	def, tool, e := compiledBuiltin("wallet", name)
	if e != nil {
		t.Fatal(e)
	}
	var parsed any
	if e = decodeStrict([]byte(args), &parsed); e != nil {
		t.Fatal(e)
	}
	return BuiltinInvocation{Context: BuiltinContext{CallID: "call", DispatchID: "dispatch", ToolID: tool.ToolID, VersionID: def.VersionID, GrantID: "grant", InputDigest: digest(parsed)}, Owner: Principal{"7", "7"}, ServiceKey: "wallet", ToolName: name, Arguments: json.RawMessage(args)}
}
func TestCompiledBuiltinCatalogIsImmutableAndSchemasValidate(t *testing.T) {
	seen := map[string]bool{}
	count := 0
	for _, d := range CompiledBuiltinDefinitions() {
		for _, tool := range d.Tools {
			count++
			if seen[tool.ToolID] {
				t.Fatal("duplicate compiled tool")
			}
			seen[tool.ToolID] = true
			if _, e := readSchema(json.RawMessage(tool.InputSchema), 0); e != nil {
				t.Fatalf("%s: %v", tool.Name, e)
			}
			if tool.PriceQuota != 0 || tool.VersionID != d.VersionID || tool.InputSchemaDigest == "" {
				t.Fatal("broken compiled basis")
			}
		}
	}
	if count != 29 {
		t.Fatalf("got %d tools", count)
	}
	a := CompiledBuiltinDefinitions()
	a[0].Tools[0].Name = "mutated"
	if CompiledBuiltinDefinitions()[0].Tools[0].Name == "mutated" {
		t.Fatal("registry aliases caller memory")
	}
}
func TestCompiledBuiltinRejectsVersionSchemaAndDigestBeforeGate(t *testing.T) {
	for _, mutate := range []func(*BuiltinInvocation){func(in *BuiltinInvocation) { in.Context.VersionID = "other" }, func(in *BuiltinInvocation) { in.Arguments = json.RawMessage(`{"payer":"attacker"}`) }, func(in *BuiltinInvocation) { in.Context.InputDigest = "other" }} {
		gate := &builtinGateStub{}
		d := &BuiltinDispatcher{Gate: gate}
		in := builtinTestInvocation(t, "wallet.balance", `{}`)
		mutate(&in)
		if _, e := d.InvokeBuiltin(context.Background(), "synthetic-personal-session", in); e == nil {
			t.Fatal("substitution accepted")
		}
		if gate.calls != 0 {
			t.Fatal("invalid input reached authority")
		}
	}
}
func TestCompiledBuiltinRequiresMatchingCoreDispatchAndRealDomain(t *testing.T) {
	in := builtinTestInvocation(t, "wallet.balance", `{}`)
	gate := &builtinGateStub{alter: true}
	d := &BuiltinDispatcher{Gate: gate}
	if _, e := d.InvokeBuiltin(context.Background(), "synthetic-personal-session", in); e != ErrDenied {
		t.Fatalf("wrong bound dispatch: %v", e)
	}
	gate.alter = false
	if _, e := d.InvokeBuiltin(context.Background(), "synthetic-personal-session", in); e != ErrUnavailable {
		t.Fatalf("missing wallet authority: %v", e)
	}
}
func TestCompiledBuiltinQueryWithoutOriginalReferenceDoesNotInvoke(t *testing.T) {
	gate := &builtinGateStub{}
	d := &BuiltinDispatcher{Gate: gate}
	in := builtinTestInvocation(t, "wallet.transfer.create", `{"quota":10}`)
	if _, e := d.QueryBuiltin(context.Background(), "synthetic-personal-session", in); e != ErrUnavailable {
		t.Fatalf("%v", e)
	}
	if gate.calls != 0 {
		t.Fatal("query invoked execution gate")
	}
}

type builtinWalletObserver struct {
	wallet.Authority
	prepares, queries int
	originalKey       string
	substitute        bool
}

func (a *builtinWalletObserver) Balance(context.Context, string) (wallet.Balance, error) {
	return wallet.Balance{OwnerUserID: 7, AccountID: 9, CreditsPerUSD: 500000}, nil
}
func (a *builtinWalletObserver) Prepare(context.Context, string, wallet.PrepareInput) (wallet.Transfer, error) {
	a.prepares++
	return wallet.Transfer{}, wallet.ErrUnknown
}
func (a *builtinWalletObserver) QueryTransferByRequestKey(_ context.Context, _ string, key string) (wallet.Transfer, error) {
	a.queries++
	a.originalKey = key
	t := wallet.Transfer{ID: 42, RequestKey: key, AmountCredits: 10, State: "pending"}
	if a.substitute {
		t.AmountCredits = 11
	}
	return t, nil
}

type builtinWalletLinks struct{ wallet.Records }

func (builtinWalletLinks) Token(context.Context, int64, string) (string, error) {
	return strings.Repeat("a", 64), nil
}
func TestCompiledBuiltinLostCreateResponseQueriesOriginalKey(t *testing.T) {
	authority := &builtinWalletObserver{}
	service, e := wallet.New(authority, builtinWalletLinks{}, "https://core.example.test")
	if e != nil {
		t.Fatal(e)
	}
	gate := &builtinGateStub{}
	d := &BuiltinDispatcher{Gate: gate, Wallet: service}
	in := builtinTestInvocation(t, "wallet.transfer.create", `{"quota":10}`)
	result, e := d.QueryBuiltin(context.Background(), "synthetic-personal-session", in)
	if e != nil || result.DownstreamOperationID != "42" || result.ExecutionStatus != "completed" || result.SettlementStatus != "settled" {
		t.Fatalf("%+v %v", result, e)
	}
	if authority.prepares != 0 || authority.queries != 1 || authority.originalKey != digest([]string{"builtin/v1", in.Context.CallID, in.Context.ToolID}) || gate.calls != 0 {
		t.Fatal("recovery retried or changed original operation")
	}
	authority.substitute = true
	if _, e = d.QueryBuiltin(context.Background(), "synthetic-personal-session", in); e != ErrConflict {
		t.Fatalf("replacement amount accepted: %v", e)
	}
}

func (g *builtinGateStub) ReadyBuiltins(context.Context) error { return nil }
func TestCompiledBuiltinReadinessCannotUseOnlyRegistryOrClientPresence(t *testing.T) {
	d := &BuiltinDispatcher{Gate: &builtinGateStub{}}
	if d.LegacyBuiltinReady() {
		t.Fatal("new clients advertised readiness")
	}
	if d.CheckReadiness(context.Background()) == nil || d.LegacyBuiltinReady() {
		t.Fatal("partial authority ACK advertised all builtins")
	}
}
