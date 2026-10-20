package app

import (
	"github.com/BurntSushi/toml"
	"testing"
)

func TestToolMarketBuiltinTOMLUsesExplicitPurposeFields(t *testing.T) {
	var cfg GoConfig
	meta, err := toml.Decode(`[toolmarket.builtins]
core_public_origin="https://api.example.test"
wallet_core_socket="/run/wallet.sock"
wallet_core_token_file="/private/wallet"
bounty_core_socket="/run/bounty.sock"
bounty_core_token_file="/private/bounty"
image_core_socket="/run/image.sock"
image_core_token_file="/private/image"
`, &cfg)
	if err != nil || len(meta.Undecoded()) != 0 {
		t.Fatal(err, meta.Undecoded())
	}
	for _, key := range meta.Keys() {
		if !toolMarketConfigurationKey(key.String()) {
			t.Fatal("purpose field absent from config whitelist", key)
		}
	}
	if err := cfg.ToolMarket.Builtins.validate(); err != nil {
		t.Fatal(err)
	}
	mapped := cfg.ToolMarket.Builtins.composition()
	if mapped.WalletSocket != "/run/wallet.sock" || mapped.BountyTokenFile != "/private/bounty" || mapped.ImageSocket != "/run/image.sock" {
		t.Fatal("purpose configuration mapping changed")
	}
	for _, bad := range []ToolMarketBuiltinsConfig{{CorePublicOrigin: "https://api.example.test"}, {WalletSocket: "/run/wallet.sock"}} {
		if bad.validate() == nil {
			t.Fatal("partial purpose configuration accepted")
		}
	}
}
