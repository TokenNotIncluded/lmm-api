package composition

import (
	"context"
	"database/sql"
	"strings"
	"testing"

	"github.com/TokenNotIncluded/api.lmm.best/extensions/internal/modules/toolmarket"
)

type builtinFactoryGate struct{}

func (builtinFactoryGate) AuthorizeBuiltin(context.Context, string, toolmarket.BuiltinAuthorizationRequest) (toolmarket.BuiltinAuthorization, error) {
	panic("configuration must not authorize a call")
}
func TestBuiltinFactoryRejectsMissingOrSharedAuthorityBeforeDatabase(t *testing.T) {
	if _, _, e := composeBuiltinDispatcher(context.Background(), BuiltinDependencies{}); e == nil {
		t.Fatal("unconfigured builtin runtime accepted")
	}
	deps := BuiltinDependencies{Database: new(sql.DB), ExpectedDatabase: "synthetic_extensions", Gate: builtinFactoryGate{}, WalletSocket: "/tmp/wallet.sock", BountySocket: "/tmp/bounty.sock", ImageSocket: "/tmp/image.sock", WalletToken: []byte(strings.Repeat("w", 32)), BountyToken: []byte(strings.Repeat("b", 32)), ImageToken: []byte(strings.Repeat("i", 32))}
	for _, change := range []func(*BuiltinDependencies){func(d *BuiltinDependencies) { d.BountyToken = d.WalletToken }, func(d *BuiltinDependencies) { d.ImageSocket = d.WalletSocket }, func(d *BuiltinDependencies) { d.ImageToken = []byte(strings.Repeat(" ", 32)) }} {
		candidate := deps
		change(&candidate)
		if _, _, e := composeBuiltinDispatcher(context.Background(), candidate); e == nil {
			t.Fatal("shared or malformed authority accepted")
		}
	}
}
