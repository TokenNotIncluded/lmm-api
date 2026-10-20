package app

import (
	"errors"
	"github.com/TokenNotIncluded/api.lmm.best/extensions/internal/composition"
	"net/url"
)

type ToolMarketConfig struct {
	CorePublicOrigin      string                   `toml:"core_public_origin"`
	FixedWitnessSocket    string                   `toml:"fixed_witness_core_socket"`
	FixedWitnessTokenFile string                   `toml:"fixed_witness_core_token_file"`
	Builtins              ToolMarketBuiltinsConfig `toml:"builtins"`
	MeteringConfigFile    string                   `toml:"metering_config_file"`
	DatabaseName          string                   `toml:"database_name"`
	DatabaseURLFile       string                   `toml:"database_url_file"`
	EncryptionKeyFile     string                   `toml:"encryption_key_file"`
	CallbackURL           string                   `toml:"callback_url"`
	CoreSocket            string                   `toml:"core_socket"`
	CoreTokenFile         string                   `toml:"core_token_file"`
}

func toolMarketConfigurationKey(key string) bool {
	switch key {
	case "toolmarket.core_public_origin":
		return true
	case "toolmarket.fixed_witness_core_socket", "toolmarket.fixed_witness_core_token_file":
		return true
	case "toolmarket.builtins", "toolmarket.builtins.core_public_origin", "toolmarket.builtins.wallet_core_socket", "toolmarket.builtins.wallet_core_token_file", "toolmarket.builtins.bounty_core_socket", "toolmarket.builtins.bounty_core_token_file", "toolmarket.builtins.image_core_socket", "toolmarket.builtins.image_core_token_file":
		return true
	case "toolmarket.metering_config_file", "toolmarket", "toolmarket.database_name", "toolmarket.database_url_file", "toolmarket.encryption_key_file", "toolmarket.callback_url", "toolmarket.core_socket", "toolmarket.core_token_file":
		return true
	}
	return false
}
func validateToolMarketConfiguration(cfg GoConfig) error {
	c := cfg.ToolMarket
	if c.CorePublicOrigin != "" {
		if cfg.Host.PublicOrigin == "" {
			return errors.New("market MCP requires an explicit Go public origin")
		}
		if err := composition.ValidateCorePublicOrigin(cfg.Host.LocalHTTP, cfg.Host.Listen, cfg.Host.PublicOrigin, c.CorePublicOrigin); err != nil {
			return err
		}
		if c.Builtins.configured() && c.Builtins.CorePublicOrigin != c.CorePublicOrigin {
			return errors.New("market Core origins must match")
		}
	}
	if (c.FixedWitnessSocket == "") != (c.FixedWitnessTokenFile == "") || c.FixedWitnessSocket != "" && (!absolutePath(c.FixedWitnessSocket) || !absolutePath(c.FixedWitnessTokenFile)) {
		return errors.New("toolmarket fixed witness requires an explicit private purpose pair")
	}
	if err := c.Builtins.validate(cfg); err != nil {
		return err
	}
	selected := false
	for _, name := range cfg.Host.Modules {
		selected = selected || name == "toolmarket"
	}
	configured := c.CorePublicOrigin != "" || c.FixedWitnessSocket != "" || c.FixedWitnessTokenFile != "" || c.Builtins.configured() || c.MeteringConfigFile != "" || c.DatabaseName != "" || c.DatabaseURLFile != "" || c.EncryptionKeyFile != "" || c.CallbackURL != "" || c.CoreSocket != "" || c.CoreTokenFile != ""
	if !selected {
		if configured {
			return errors.New("toolmarket configuration requires explicit module selection")
		}
		return nil
	}
	if c.MeteringConfigFile != "" && !absolutePath(c.MeteringConfigFile) {
		return errors.New("toolmarket metering configuration path must be absolute")
	}
	if c.DatabaseName == "" || !absolutePath(c.DatabaseURLFile) || !absolutePath(c.EncryptionKeyFile) || !absolutePath(c.CoreSocket) || !absolutePath(c.CoreTokenFile) {
		return errors.New("toolmarket requires dedicated database, private key, Core socket and credential")
	}
	u, err := url.Parse(c.CallbackURL)
	validScheme := err == nil && u.Scheme == "https"
	if cfg.Host.LocalHTTP {
		public, publicErr := url.Parse(cfg.Host.PublicOrigin)
		validScheme = err == nil && publicErr == nil && validateHostProfile(cfg) == nil && u.Scheme == "http" && u.Host == public.Host
	}
	if err != nil || !validScheme || u.Hostname() == "" || u.User != nil || u.RawQuery != "" || u.ForceQuery || u.Fragment != "" {
		return errors.New("toolmarket requires fixed public HTTPS callback")
	}
	for _, other := range []string{cfg.Host.TokenFile, cfg.Core.IdentityTokenFile, cfg.Core.CommerceTokenFile, cfg.Core.PaymentTokenFile, cfg.Core.AssistantTokenFile, c.EncryptionKeyFile} {
		if c.CoreTokenFile == other {
			return errors.New("toolmarket Core credential must be independent")
		}
	}
	if c.CoreSocket == cfg.Core.Socket {
		return errors.New("toolmarket requires a dedicated internal Unix socket")
	}
	return nil
}
