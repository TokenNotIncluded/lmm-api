package composition

import "testing"

func TestLocalHTTPProfileKeepsGoOriginAndCorePortsIsolated(t *testing.T) {
	if err := ValidateHostProfile(false, "0.0.0.0:8000", ""); err != nil {
		t.Fatal(err)
	}
	if err := ValidateHostProfile(true, "127.0.0.1:39001", "http://127.0.0.1:39001"); err != nil {
		t.Fatal(err)
	}
	for _, pair := range [][2]string{{"0.0.0.0:39001", "http://0.0.0.0:39001"}, {"localhost:39001", "http://localhost:39001"}, {"127.0.0.1:80", "http://127.0.0.1:80"}, {"127.0.0.1:39001", "http://127.0.0.1:39002"}, {"127.0.0.1:39001", "https://127.0.0.1:39001"}, {"127.0.0.1:39001", "http://127.0.0.1:39001/path"}} {
		if ValidateHostProfile(true, pair[0], pair[1]) == nil {
			t.Fatal("invalid fixture profile", pair)
		}
	}
	if err := ValidateCorePublicOrigin(true, "127.0.0.1:39001", "http://127.0.0.1:39001", "http://127.0.0.1:39002"); err != nil {
		t.Fatal("separate Core listener rejected", err)
	}
	for _, origin := range []string{"http://127.0.0.2:39002", "http://localhost:39002", "http://127.0.0.1:80", "https://api.example.test"} {
		if ValidateCorePublicOrigin(true, "127.0.0.1:39001", "http://127.0.0.1:39001", origin) == nil {
			t.Fatal("fixture escaped local Core", origin)
		}
	}
	if ValidateCorePublicOrigin(false, "", "", "http://127.0.0.1:39002") == nil {
		t.Fatal("production allowed local HTTP")
	}
	if validateToolMarketCallbackProfile(Config{LocalHTTP: true, HostListen: "127.0.0.1:39001", HostPublicOrigin: "http://127.0.0.1:39001", ToolMarketCallbackURL: "http://127.0.0.1:39002/callback"}) == nil {
		t.Fatal("callback routed to another process")
	}
}

func TestMarketMCPProfileRequiresConfiguredTrustedOrigins(t *testing.T) {
	cfg := Config{LocalHTTP: true, HostListen: "127.0.0.1:39001", HostPublicOrigin: "http://127.0.0.1:39001", ToolMarketCorePublicOrigin: "http://127.0.0.1:39002"}
	// This reaches the explicit module-selection gate, proving the distinct Core
	// port passed profile validation without needing storage or credentials.
	if err := validateToolMarketSelection(cfg); err == nil || err.Error() != "toolmarket configuration requires explicit selection" {
		t.Fatal("valid pair did not reach selection gate", err)
	}
	for _, core := range []string{"http://127.0.0.1:39002/", "http://127.0.0.1:39002?", "http://127.0.0.1:39002#x", "http://user@127.0.0.1:39002", "http://127.0.0.2:39002"} {
		bad := cfg
		bad.ToolMarketCorePublicOrigin = core
		if err := validateToolMarketSelection(bad); err == nil || err.Error() == "toolmarket configuration requires explicit selection" {
			t.Fatal("untrusted Core origin reached selection gate", core, err)
		}
	}
	cfg.HostPublicOrigin = ""
	if err := validateToolMarketSelection(cfg); err == nil || err.Error() != "market MCP requires a configured Go public origin" {
		t.Fatal("implicit resource origin accepted", err)
	}
}
