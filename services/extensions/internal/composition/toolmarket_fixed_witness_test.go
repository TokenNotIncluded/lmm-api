package composition

import "testing"

func TestFixedWitnessPurposeIsOptionalPairedAndIndependent(t *testing.T) {
	base := Config{Socket: "/run/identity.sock", ToolMarketSocket: "/run/market.sock", ToolMarketTokenFile: "/private/market", ToolMarketKeyFile: "/private/key"}
	if err := validateToolMarketWitness(base); err != nil {
		t.Fatal(err)
	}
	valid := base
	valid.ToolMarketWitnessSocket = base.ToolMarketSocket
	valid.ToolMarketWitnessTokenFile = "/private/witness"
	if err := validateToolMarketWitness(valid); err != nil {
		t.Fatal("independent purpose on reviewed market UDS rejected", err)
	}
	for _, change := range []func(*Config){func(c *Config) { c.ToolMarketWitnessSocket = "" }, func(c *Config) { c.ToolMarketWitnessTokenFile = "" }, func(c *Config) { c.ToolMarketWitnessSocket = base.Socket }, func(c *Config) { c.ToolMarketWitnessTokenFile = base.ToolMarketTokenFile }, func(c *Config) { c.ToolMarketWitnessTokenFile = base.ToolMarketKeyFile }} {
		bad := valid
		change(&bad)
		if validateToolMarketWitness(bad) == nil {
			t.Fatal("unscoped purpose configuration accepted")
		}
	}
}
