package composition

import (
	"context"
	"database/sql"
	"strings"
	"testing"

	"github.com/TokenNotIncluded/lmm-api/services/extensions/internal/modules/toolmarket"
)

type builtinFactoryGate struct{}

func (builtinFactoryGate) AuthorizeBuiltin(context.Context, string, toolmarket.BuiltinAuthorizationRequest) (toolmarket.BuiltinAuthorization, error) {
	panic("configuration must not authorize a call")
}
func TestBuiltinFactoryRejectsMissingOrSharedAuthorityBeforeDatabase(t *testing.T) {
	if _, _, e := composeBuiltinDispatcher(context.Background(), BuiltinDependencies{}); e == nil {
		t.Fatal("unconfigured builtin runtime accepted")
	}
	deps := BuiltinDependencies{Database: new(sql.DB), ExpectedDatabase: "synthetic_extensions", Gate: builtinFactoryGate{}, WalletSocket: "/tmp/core-private.sock", BountySocket: "/tmp/core-private.sock", ImageSocket: "/tmp/core-private.sock", WalletToken: []byte(strings.Repeat("w", 32)), BountyToken: []byte(strings.Repeat("b", 32)), ImageToken: []byte(strings.Repeat("i", 32))}
	for _, change := range []func(*BuiltinDependencies){func(d *BuiltinDependencies) { d.BountyToken = d.WalletToken }, func(d *BuiltinDependencies) { d.ImageSocket = "/tmp/different.sock" }, func(d *BuiltinDependencies) { d.ImageSocket = "relative" }, func(d *BuiltinDependencies) { d.ImageToken = []byte(strings.Repeat(" ", 32)) }} {
		candidate := deps
		change(&candidate)
		if _, _, e := composeBuiltinDispatcher(context.Background(), candidate); e == nil {
			t.Fatal("shared or malformed authority accepted")
		}
	}
	// A closed database cannot perform I/O. Reaching the storage readiness error
	// demonstrates that one socket with independent tokens passes both guards.
	db, err := sql.Open("postgres", "host=/nonexistent")
	if err != nil {
		t.Fatal(err)
	}
	if err := db.Close(); err != nil {
		t.Fatal(err)
	}
	deps.Database = db
	if _, _, err := composeBuiltinDispatcher(context.Background(), deps); err == nil || err.Error() != "wallet records installation unavailable" {
		t.Fatal("valid shared-socket configuration did not reach storage readiness", err)
	}
}
