package composition

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestSupportCustomerSelectionAndIndependentSecrets(t *testing.T) {
	if validateSupportCustomersSelection(Config{SupportCustomerSocket: "/run/private.sock"}) == nil {
		t.Fatal("ignored partial/unselected config")
	}
	if _, err := New(context.Background(), Config{SupportCustomerSocket: "/run/private.sock"}); err == nil {
		t.Fatal("empty module selection ignored configured customer projection")
	}
	if validateSupportCustomersSelection(Config{}) != nil {
		t.Fatal("optional projection required")
	}
	cfg := Config{Selected: []string{"support"}, Socket: "/run/core.sock", SupportCustomerSocket: "/run/private.sock", SupportCustomerTokenFile: "/private/support-token", SupportNoteKeyFile: "/private/support-key"}
	if err := validateSupportCustomersSelection(cfg); err != nil {
		t.Fatal(err)
	}
	root := t.TempDir()
	cfg.HostCredential = []byte(strings.Repeat("h", 32))
	cfg.SupportCustomerTokenFile = filepath.Join(root, "token")
	cfg.SupportNoteKeyFile = filepath.Join(root, "key")
	key := []byte(strings.Repeat("k", 32))
	if err := os.WriteFile(cfg.SupportNoteKeyFile, key, 0600); err != nil {
		t.Fatal(err)
	}
	identity := []byte(strings.Repeat("i", 32))
	for _, aliased := range [][]byte{cfg.HostCredential, key, identity} {
		if err := os.WriteFile(cfg.SupportCustomerTokenFile, aliased, 0600); err != nil {
			t.Fatal(err)
		}
		if _, _, err := loadSupportCustomerCredentials(cfg, identity); err == nil {
			t.Fatal("aliased credential accepted")
		}
	}
	wanted := []byte(strings.Repeat("s", 32))
	if err := os.WriteFile(cfg.SupportCustomerTokenFile, wanted, 0600); err != nil {
		t.Fatal(err)
	}
	token, loaded, err := loadSupportCustomerCredentials(cfg, identity)
	if err != nil || string(token) != string(wanted) || string(loaded) != string(key) {
		t.Fatal("private exact bytes changed", err)
	}
	if err := os.Chmod(cfg.SupportNoteKeyFile, 0644); err != nil {
		t.Fatal(err)
	}
	if _, _, err := loadSupportCustomerCredentials(cfg, identity); err == nil {
		t.Fatal("public note key accepted")
	}
}
