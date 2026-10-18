package app

import "errors"

func validateAssistantConfiguration(cfg GoConfig) error {
	key := cfg.Core.AssistantTokenFile
	if key == "" {
		return nil
	}
	selected := false
	for _, name := range cfg.Host.Modules {
		selected = selected || name == "assistant"
	}
	if !selected {
		return errors.New("assistant RPC credential requires explicit assistant selection")
	}
	if !absolutePath(key) {
		return errors.New("assistant RPC credential path must be absolute")
	}
	for _, other := range []string{cfg.Host.TokenFile, cfg.Core.IdentityTokenFile, cfg.Core.CommerceTokenFile, cfg.Core.PaymentTokenFile, cfg.Assistant.CursorKeyFile} {
		if other == key {
			return errors.New("assistant RPC credential must be independent")
		}
	}
	return nil
}
