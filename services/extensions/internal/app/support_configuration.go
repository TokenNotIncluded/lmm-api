package app

import "errors"

type SupportCustomersConfig struct {
	CoreSocket    string `toml:"core_socket"`
	CoreTokenFile string `toml:"core_token_file"`
	NoteKeyFile   string `toml:"note_key_file"`
}

func supportCustomersConfigurationKey(key string) bool {
	switch key {
	case "support_customers", "support_customers.core_socket", "support_customers.core_token_file", "support_customers.note_key_file":
		return true
	}
	return false
}
func validateSupportCustomersConfiguration(cfg GoConfig) error {
	c := cfg.SupportCustomers
	if c.CoreSocket == "" && c.CoreTokenFile == "" && c.NoteKeyFile == "" {
		return nil
	}
	selected := false
	for _, name := range cfg.Host.Modules {
		selected = selected || name == "support"
	}
	if !selected || !absolutePath(c.CoreSocket) || !absolutePath(c.CoreTokenFile) || !absolutePath(c.NoteKeyFile) || c.CoreSocket == cfg.Core.Socket {
		return errors.New("support customers require explicit support selection and private HTTP socket with absolute secret paths")
	}
	if c.CoreTokenFile == c.NoteKeyFile {
		return errors.New("support note key and Core credential must be independent")
	}
	for _, path := range []string{cfg.Host.TokenFile, cfg.Core.IdentityTokenFile, cfg.Core.CommerceTokenFile, cfg.Core.PaymentTokenFile, cfg.Core.AssistantTokenFile, cfg.ToolMarket.EncryptionKeyFile, cfg.ToolMarket.CoreTokenFile, cfg.Assistant.CursorKeyFile} {
		if path != "" && (path == c.CoreTokenFile || path == c.NoteKeyFile) {
			return errors.New("support projection secrets must be independent")
		}
	}
	return nil
}
