package app

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestAssistantFullsourceCanonicalTokenConfigUsesRealLoader(t *testing.T) {
	raw := "version=1\n[host]\ntoken_file='/tmp/host-token'\nmodules=['assistant']\n[core]\nsocket='/tmp/core.sock'\nidentity_token_file='/tmp/identity-token'\nassistant_token_file='/tmp/assistant-token'\n[assistant]\ndatabase_name='lmm_assistant_private_test'\ndatabase_url_file='/tmp/assistant-url'\ncursor_key_file='/tmp/assistant-cursor'\n"
	write := func(body string) string {
		path := filepath.Join(t.TempDir(), "extensions.toml")
		if os.WriteFile(path, []byte(body), 0o600) != nil {
			t.Fatal("cannot write private synthetic config")
		}
		return path
	}
	cfg, err := LoadConfig(write(raw))
	if err != nil || cfg.Core.AssistantTokenFile != "/tmp/assistant-token" || cfg.Core.IdentityTokenFile != "/tmp/identity-token" {
		t.Fatal("canonical assistant credential did not pass the real typed loader")
	}
	for _, body := range []string{
		strings.Replace(raw, "assistant_token_file=", "AssistantTokenFile=", 1),
		strings.Replace(raw, "/tmp/assistant-token", "relative-token", 1),
		strings.Replace(raw, "/tmp/assistant-token", "/tmp/identity-token", 1),
		strings.Replace(raw, "modules=['assistant']", "modules=['identity']", 1),
	} {
		if _, err := LoadConfig(write(body)); err == nil {
			t.Fatal("real loader accepted an ignored or aliased assistant credential")
		}
	}
	without := strings.Replace(raw, "assistant_token_file='/tmp/assistant-token'\n", "", 1)
	if cfg, err := LoadConfig(write(without)); err != nil || cfg.Core.AssistantTokenFile != "" {
		t.Fatal("missing optional assistant credential created a fallback or rejected history configuration")
	}
}
