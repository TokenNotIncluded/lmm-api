package composition

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func builtinHostConfig() ToolMarketBuiltinConfig {
	return ToolMarketBuiltinConfig{CorePublicOrigin: "https://api.example.test", WalletSocket: "/run/wallet.sock", WalletTokenFile: "/private/wallet", BountySocket: "/run/bounty.sock", BountyTokenFile: "/private/bounty", ImageSocket: "/run/image.sock", ImageTokenFile: "/private/image"}
}
func TestToolMarketBuiltinConfigurationIsCompleteAndIndependent(t *testing.T) {
	valid := builtinHostConfig()
	base := Config{Socket: "/run/core.sock", TokenFile: "/private/identity", ToolMarketSocket: "/run/toolmarket.sock", ToolMarketTokenFile: "/private/toolmarket"}
	if err := valid.validate(base); err != nil {
		t.Fatal(err)
	}
	for _, bad := range []ToolMarketBuiltinConfig{{CorePublicOrigin: valid.CorePublicOrigin}, {WalletSocket: valid.WalletSocket}} {
		if bad.validate(base) == nil {
			t.Fatal("partial builtin configuration accepted")
		}
	}
	for _, origin := range []string{"http://api.example.test", "https://localhost", "https://127.0.0.1", "https://user@api.example.test", "https://api.example.test/path", "https://api.example.test?x=1"} {
		c := valid
		c.CorePublicOrigin = origin
		if c.validate(base) == nil {
			t.Fatal("unsafe public origin", origin)
		}
	}
	for _, path := range []string{valid.BountyTokenFile, base.TokenFile, base.ToolMarketTokenFile, "relative", "/bad\npath"} {
		c := valid
		c.WalletTokenFile = path
		if c.validate(base) == nil {
			t.Fatal("aliased private credential", path)
		}
	}
	for _, socket := range []string{valid.BountySocket, base.Socket, base.ToolMarketSocket, "relative"} {
		c := valid
		c.WalletSocket = socket
		if c.validate(base) == nil {
			t.Fatal("aliased authority socket", socket)
		}
	}
}
func TestBuiltinTokenContentAliasingIsRejectedBeforeDatabaseOpen(t *testing.T) {
	root := t.TempDir()
	config := builtinHostConfig()
	write := func(name, value string) string {
		path := filepath.Join(root, name)
		if err := os.WriteFile(path, []byte(strings.Repeat(value, 32)), 0600); err != nil {
			t.Fatal(err)
		}
		return path
	}
	config.WalletTokenFile = write("wallet", "w")
	config.BountyTokenFile = write("bounty", "b")
	config.ImageTokenFile = write("image", "i")
	cfg := Config{HostCredential: []byte(strings.Repeat("h", 32)), ToolMarketBuiltins: config, ToolMarketTokenFile: write("market", "m"), ToolMarketKeyFile: write("key", "k")}
	runtime := &Runtime{}
	err := runtime.composeToolMarket(context.Background(), cfg, []byte(strings.Repeat("i", 32)))
	if err == nil || !strings.Contains(err.Error(), "independent") || len(runtime.close) != 0 || runtime.ToolMarket != nil {
		t.Fatal("aliased purpose reached database", err)
	}
}
