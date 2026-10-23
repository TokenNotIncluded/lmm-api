package composition

import (
	"errors"
	"path/filepath"
	"strings"

	"github.com/TokenNotIncluded/lmm-api/services/extensions/internal/coreclient"
)

// ToolMarketBuiltinConfig selects purpose-scoped authorities on one Core UDS. No
// supplier verification key, Core DSN, or runtime schema installer is accepted.
type ToolMarketBuiltinConfig struct {
	CorePublicOrigin              string
	WalletSocket, WalletTokenFile string
	BountySocket, BountyTokenFile string
	ImageSocket, ImageTokenFile   string
}

func (c ToolMarketBuiltinConfig) configured() bool { return c != (ToolMarketBuiltinConfig{}) }
func (c ToolMarketBuiltinConfig) validate(cfg Config) error {
	if !c.configured() {
		return nil
	}
	if err := ValidateCorePublicOrigin(cfg.LocalHTTP, cfg.HostListen, cfg.HostPublicOrigin, c.CorePublicOrigin); err != nil {
		return err
	}
	sockets := []string{c.WalletSocket, c.BountySocket, c.ImageSocket}
	paths := []string{c.WalletTokenFile, c.BountyTokenFile, c.ImageTokenFile}
	if cfg.ToolMarketSocket == "" || cfg.ToolMarketSocket == cfg.Socket || cfg.SupportCustomerSocket != "" && cfg.SupportCustomerSocket != cfg.ToolMarketSocket {
		return errors.New("builtin authorities require the shared Core private Unix socket")
	}
	for i, p := range paths {
		if !filepath.IsAbs(p) || filepath.Clean(p) != p || strings.ContainsAny(p, "\x00\r\n") {
			return errors.New("builtin purpose credentials require absolute private paths")
		}
		for _, other := range []string{cfg.TokenFile, cfg.CommerceTokenFile, cfg.PaymentTokenFile, cfg.AssistantTokenFile, cfg.SupportCustomerTokenFile, cfg.SupportNoteKeyFile, cfg.ToolMarketTokenFile, cfg.ToolMarketKeyFile, cfg.ToolMarketWitnessTokenFile} {
			if p == other {
				return errors.New("builtin purpose credential paths must be independent")
			}
		}
		if !filepath.IsAbs(sockets[i]) || filepath.Clean(sockets[i]) != sockets[i] || strings.ContainsAny(sockets[i], "\x00\r\n") || sockets[i] != cfg.ToolMarketSocket {
			return errors.New("builtin authorities must share the Core private Unix socket")
		}
		for j := 0; j < i; j++ {
			if p == paths[j] {
				return errors.New("builtin purpose credential paths must be independent")
			}
		}
	}
	return nil
}
func loadToolMarketBuiltinTokens(cfg ToolMarketBuiltinConfig) ([][]byte, error) {
	if !cfg.configured() {
		return nil, nil
	}
	var tokens [][]byte
	for _, path := range []string{cfg.WalletTokenFile, cfg.BountyTokenFile, cfg.ImageTokenFile} {
		token, err := coreclient.ReadTokenFile(path)
		if err != nil {
			return nil, errors.New("private builtin purpose credential unavailable")
		}
		tokens = append(tokens, token)
	}
	return tokens, nil
}
