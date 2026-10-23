package composition

import (
	"errors"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/coreclient"
	"path/filepath"
	"strings"
)

func validateToolMarketWitness(cfg Config) error {
	socket, path := cfg.ToolMarketWitnessSocket, cfg.ToolMarketWitnessTokenFile
	if socket == "" && path == "" {
		return nil
	}
	for _, p := range []string{socket, path} {
		if !filepath.IsAbs(p) || filepath.Clean(p) != p || strings.ContainsAny(p, "\x00\r\n") {
			return errors.New("fixed witness requires a private purpose socket and credential")
		}
	}
	if socket == cfg.Socket {
		return errors.New("fixed witness cannot use identity socket")
	}
	for _, other := range []string{cfg.TokenFile, cfg.CommerceTokenFile, cfg.PaymentTokenFile, cfg.AssistantTokenFile, cfg.SupportCustomerTokenFile, cfg.SupportNoteKeyFile, cfg.ToolMarketTokenFile, cfg.ToolMarketKeyFile, cfg.ToolMarketBuiltins.WalletTokenFile, cfg.ToolMarketBuiltins.BountyTokenFile, cfg.ToolMarketBuiltins.ImageTokenFile} {
		if path == other {
			return errors.New("fixed witness purpose credential must be independent")
		}
	}
	return nil
}
func loadToolMarketWitness(cfg Config) ([]byte, error) {
	if cfg.ToolMarketWitnessTokenFile == "" {
		return nil, nil
	}
	token, err := coreclient.ReadTokenFile(cfg.ToolMarketWitnessTokenFile)
	if err != nil {
		return nil, errors.New("private fixed witness credential unavailable")
	}
	return token, nil
}
