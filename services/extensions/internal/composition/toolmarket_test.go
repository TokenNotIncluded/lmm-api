package composition

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestToolMarketSelectionAndCredentialIsolationBeforeDatabaseOpen(t *testing.T) {
	if err := validateToolMarketSelection(Config{Selected: []string{"toolmarket"}, ToolMarketMeteringFile: "/missing/operator-key.toml"}); err == nil || !strings.Contains(err.Error(), "configured in Core") {
		t.Fatal("Go host accepted supplier authority keys", err)
	}
	if validateToolMarketSelection(Config{ToolMarketSocket: "/run/tool.sock"}) == nil {
		t.Fatal("ignored unselected toolmarket configuration")
	}
	if validateToolMarketSelection(Config{Selected: []string{"toolmarket"}}) == nil {
		t.Fatal("selected toolmarket had no runtime configuration")
	}
	for _, role := range []string{"host", "identity", "commerce", "payment", "assistant", "key"} {
		t.Run(role, func(t *testing.T) {
			root := t.TempDir()
			key := []byte(strings.Repeat("k", 32))
			host := []byte(strings.Repeat("h", 32))
			other := [][]byte{[]byte(strings.Repeat("i", 32)), []byte(strings.Repeat("c", 32)), []byte(strings.Repeat("p", 32)), []byte(strings.Repeat("a", 32))}
			token := []byte(strings.Repeat("t", 32))
			switch role {
			case "host":
				token = host
			case "identity":
				token = other[0]
			case "commerce":
				token = other[1]
			case "payment":
				token = other[2]
			case "assistant":
				token = other[3]
			case "key":
				token = key
			}
			tokenPath, keyPath := filepath.Join(root, "token"), filepath.Join(root, "key")
			if os.WriteFile(tokenPath, token, 0600) != nil || os.WriteFile(keyPath, key, 0600) != nil {
				t.Fatal("cannot prepare synthetic private credential")
			}
			runtime := &Runtime{}
			err := runtime.composeToolMarket(context.Background(), Config{HostCredential: host, ToolMarketTokenFile: tokenPath, ToolMarketKeyFile: keyPath}, other...)
			if err == nil || !strings.Contains(err.Error(), "independent") || runtime.ToolMarket != nil || len(runtime.close) != 0 {
				t.Fatal("aliased credential reached database assembly", err)
			}
		})
	}
}
