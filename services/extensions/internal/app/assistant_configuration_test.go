package app

import "testing"

func TestAssistantConfigurationRequiresExplicitIndependentCredential(t *testing.T) {
	cfg := GoConfig{Host: HostConfig{Modules: []string{"assistant"}, TokenFile: "/tmp/host-token"}, Core: CoreConfig{AssistantTokenFile: "/tmp/assistant-token", IdentityTokenFile: "/tmp/identity-token", CommerceTokenFile: "/tmp/commerce-token", PaymentTokenFile: "/tmp/payment-token"}}
	if err := validateAssistantConfiguration(cfg); err != nil {
		t.Fatal(err)
	}
	for _, path := range []string{cfg.Host.TokenFile, cfg.Core.IdentityTokenFile, cfg.Core.CommerceTokenFile, cfg.Core.PaymentTokenFile, "relative"} {
		mutated := cfg
		mutated.Core.AssistantTokenFile = path
		if validateAssistantConfiguration(mutated) == nil {
			t.Fatal("aliased or relative assistant credential accepted")
		}
	}
	cfg.Host.Modules = nil
	if validateAssistantConfiguration(cfg) == nil {
		t.Fatal("ignored assistant credential without module selection")
	}
	cfg.Core.AssistantTokenFile = ""
	if validateAssistantConfiguration(cfg) != nil {
		t.Fatal("missing optional assistant credential stopped other modules")
	}
}
