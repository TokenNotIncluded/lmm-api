package composition

import (
	"errors"
	"path/filepath"
	"strings"

	"github.com/TokenNotIncluded/api.lmm.best/extensions/internal/coreclient"
)

// ToolMarketBuiltinConfig selects separately scoped Core authorities. No
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
	for i, p := range paths {
		if !filepath.IsAbs(p) || filepath.Clean(p) != p || strings.ContainsAny(p, "\x00\r\n") {
			return errors.New("builtin purpose credentials require absolute private paths")
		}
		for _, other := range []string{cfg.TokenFile, cfg.CommerceTokenFile, cfg.PaymentTokenFile, cfg.AssistantTokenFile, cfg.SupportCustomerTokenFile, cfg.SupportNoteKeyFile, cfg.ToolMarketTokenFile, cfg.ToolMarketKeyFile} {
			if p == other {
				return errors.New("builtin purpose credential paths must be independent")
			}
		}
		if !filepath.IsAbs(sockets[i]) || filepath.Clean(sockets[i]) != sockets[i] || strings.ContainsAny(sockets[i], "\x00\r\n") || sockets[i] == cfg.Socket || sockets[i] == cfg.ToolMarketSocket || sockets[i] == cfg.SupportCustomerSocket {
			return errors.New("builtin authorities require independent Unix sockets")
		}
		for j := 0; j < i; j++ {
			if p == paths[j] || sockets[i] == sockets[j] {
				return errors.New("builtin authority paths must be independent")
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
