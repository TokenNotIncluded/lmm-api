package app

import (
	"errors"

	"github.com/TokenNotIncluded/lmm-api/services/extensions/internal/composition"
)

type ToolMarketBuiltinsConfig struct {
	CorePublicOrigin string `toml:"core_public_origin"`
	WalletSocket     string `toml:"wallet_core_socket"`
	WalletTokenFile  string `toml:"wallet_core_token_file"`
	BountySocket     string `toml:"bounty_core_socket"`
	BountyTokenFile  string `toml:"bounty_core_token_file"`
	ImageSocket      string `toml:"image_core_socket"`
	ImageTokenFile   string `toml:"image_core_token_file"`
}

func (c ToolMarketBuiltinsConfig) configured() bool { return c != (ToolMarketBuiltinsConfig{}) }
func (c ToolMarketBuiltinsConfig) composition() composition.ToolMarketBuiltinConfig {
	return composition.ToolMarketBuiltinConfig{CorePublicOrigin: c.CorePublicOrigin, WalletSocket: c.WalletSocket, WalletTokenFile: c.WalletTokenFile, BountySocket: c.BountySocket, BountyTokenFile: c.BountyTokenFile, ImageSocket: c.ImageSocket, ImageTokenFile: c.ImageTokenFile}
}
func (c ToolMarketBuiltinsConfig) validate(profiles ...GoConfig) error {
	if !c.configured() {
		return nil
	}
	for _, p := range []string{c.WalletSocket, c.WalletTokenFile, c.BountySocket, c.BountyTokenFile, c.ImageSocket, c.ImageTokenFile} {
		if !absolutePath(p) {
			return errors.New("toolmarket builtin authorities require complete private purpose configuration")
		}
	}
	var profile GoConfig
	if len(profiles) > 0 {
		profile = profiles[0]
	}
	if err := composition.ValidateCorePublicOrigin(profile.Host.LocalHTTP, profile.Host.Listen, profile.Host.PublicOrigin, c.CorePublicOrigin); err != nil {
		return err
	}
	return nil
}
