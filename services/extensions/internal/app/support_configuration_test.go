package app

import "testing"

func TestSupportCustomerConfigurationRequiresDedicatedSecrets(t *testing.T) {
	cfg := GoConfig{Host: HostConfig{Modules: []string{"support"}, TokenFile: "/private/host"}, Core: CoreConfig{Socket: "/run/core.sock", IdentityTokenFile: "/private/identity"}, SupportCustomers: SupportCustomersConfig{CoreSocket: "/run/private.sock", CoreTokenFile: "/private/support-token", NoteKeyFile: "/private/support-key"}}
	if err := validateSupportCustomersConfiguration(cfg); err != nil {
		t.Fatal(err)
	}
	for _, path := range []string{cfg.Host.TokenFile, cfg.Core.IdentityTokenFile, cfg.SupportCustomers.NoteKeyFile, "relative"} {
		changed := cfg
		changed.SupportCustomers.CoreTokenFile = path
		if validateSupportCustomersConfiguration(changed) == nil {
			t.Fatal("shared or relative secret accepted")
		}
	}
	changed := cfg
	changed.Host.Modules = nil
	if validateSupportCustomersConfiguration(changed) == nil {
		t.Fatal("unselected support config ignored")
	}
	changed = cfg
	changed.SupportCustomers.CoreSocket = cfg.Core.Socket
	if validateSupportCustomersConfiguration(changed) == nil {
		t.Fatal("gRPC socket used for private HTTP")
	}
	if validateSupportCustomersConfiguration(GoConfig{}) != nil {
		t.Fatal("disabled support projection blocked startup")
	}
}
