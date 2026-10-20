package app

import "testing"

func TestToolMarketExplicitDedicatedConfiguration(t *testing.T) {
	cfg := GoConfig{Host: HostConfig{Modules: []string{"toolmarket"}, TokenFile: "/private/host"}, Core: CoreConfig{Socket: "/run/core.sock", IdentityTokenFile: "/private/identity", CommerceTokenFile: "/private/commerce", PaymentTokenFile: "/private/payment", AssistantTokenFile: "/private/assistant"}, ToolMarket: ToolMarketConfig{DatabaseName: "lmm_toolmarket", DatabaseURLFile: "/private/tool-db", EncryptionKeyFile: "/private/tool-key", CallbackURL: "https://app.example/api/tool-market/runtime/oauth/callback", CoreSocket: "/run/tool-market.sock", CoreTokenFile: "/private/tool-token"}}
	if err := validateToolMarketConfiguration(cfg); err != nil {
		t.Fatal(err)
	}
	for _, path := range []string{cfg.Host.TokenFile, cfg.Core.IdentityTokenFile, cfg.Core.CommerceTokenFile, cfg.Core.PaymentTokenFile, cfg.Core.AssistantTokenFile, cfg.ToolMarket.EncryptionKeyFile, "relative"} {
		changed := cfg
		changed.ToolMarket.CoreTokenFile = path
		if validateToolMarketConfiguration(changed) == nil {
			t.Fatal("shared or relative credential accepted")
		}
	}
	changed := cfg
	changed.Host.Modules = nil
	if validateToolMarketConfiguration(changed) == nil {
		t.Fatal("unselected toolmarket config ignored")
	}
	changed = cfg
	changed.ToolMarket.CoreSocket = changed.Core.Socket
	if validateToolMarketConfiguration(changed) == nil {
		t.Fatal("shared identity socket accepted")
	}
	for _, callback := range []string{"http://app.example/callback", "https://user@app.example/callback", "https://app.example/callback?redirect=x", "https://app.example/callback#fragment"} {
		changed = cfg
		changed.ToolMarket.CallbackURL = callback
		if validateToolMarketConfiguration(changed) == nil {
			t.Fatal("unfixed callback accepted")
		}
	}
	if err := validateToolMarketConfiguration(GoConfig{}); err != nil {
		t.Fatal("disabled toolmarket blocked other modules", err)
	}
}
