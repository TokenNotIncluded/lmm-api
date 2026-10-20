package app

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestHostLocalHTTPRequiresExplicitExactListener(t *testing.T) {
	base := "version=1\n[host]\ntoken_file='/tmp/fixture-host-token'\nmodules=['none']\nweb_compat=false\nlisten='127.0.0.1:39001'\npublic_origin='http://127.0.0.1:39001'\nlocal_http=true\n"
	load := func(raw string) error {
		t.Helper()
		path := filepath.Join(t.TempDir(), "extensions.toml")
		if err := os.WriteFile(path, []byte(raw), 0600); err != nil {
			t.Fatal(err)
		}
		_, err := LoadConfig(path)
		return err
	}
	if err := load(base); err != nil {
		t.Fatal(err)
	}
	for _, raw := range []string{
		strings.Replace(base, "local_http=true", "local_http=false", 1),
		strings.Replace(base, "local_http=true\n", "", 1),
		strings.Replace(base, "listen='127.0.0.1:39001'", "listen='127.0.0.1:39002'", 1),
		strings.ReplaceAll(base, "127.0.0.1", "localhost"),
		strings.ReplaceAll(base, ":39001", ":80"),
		strings.ReplaceAll(base, "local_http", "LocalHTTP"),
	} {
		if err := load(raw); err == nil {
			t.Fatal("accepted implicit or mismatched local HTTP profile")
		}
	}
}
