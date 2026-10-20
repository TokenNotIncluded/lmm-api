// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package toolmarket

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
)

func legacyMeteringFixture(t *testing.T) (string, string, LegacyMeteringBinding, string) {
	t.Helper()
	root := t.TempDir()
	key := filepath.Join(root, "collector.key")
	config := filepath.Join(root, "metering.toml")
	if err := os.WriteFile(key, []byte(strings.Repeat("synthetic-key-", 4)), 0600); err != nil {
		t.Fatal(err)
	}
	binding := LegacyMeteringBinding{ServiceID: "service", VersionID: "version", Endpoint: "https://example.com/mcp", ToolID: "tool", RemoteDigest: strings.Repeat("a", 64)}
	document := fmt.Sprintf("[[adapters]]\nservice_id = %q\nversion_id = %q\nendpoint = %q\ntool_id = %q\nremote_digest = %q\nadapter_id = \"collector\"\nkey_file = %q\nmetrics = [\"input_tokens\", \"images\"]\n", binding.ServiceID, binding.VersionID, binding.Endpoint, binding.ToolID, binding.RemoteDigest, key)
	if err := os.WriteFile(config, []byte(document), 0600); err != nil {
		t.Fatal(err)
	}
	return config, key, binding, document
}
func TestLegacyMeteringPrivateSnapshotBindingAndCopies(t *testing.T) {
	config, key, binding, _ := legacyMeteringFixture(t)
	resolver, err := LoadLegacyMeteringConfig(config)
	if err != nil {
		t.Fatal(err)
	}
	first, err := resolver.ResolveLegacyMetering(context.Background(), binding)
	if err != nil || first.AdapterID != "collector" || len(first.Key) != len(strings.Repeat("synthetic-key-", 4)) || len(first.Metrics) != 2 || first.Context != (LegacyMeteringContext{}) {
		t.Fatal("resolved configuration mismatch", err)
	}
	first.Key[0] = 0
	first.Metrics[0] = "gpu_milliseconds"
	first.Context.CallID = "caller"
	second, err := resolver.ResolveLegacyMetering(context.Background(), binding)
	if err != nil || second.Key[0] != 's' || second.Metrics[0] != "input_tokens" || second.Context.CallID != "" {
		t.Fatal("shared mutable result", err)
	}
	if err := os.WriteFile(key, []byte(strings.Repeat("z", 32)), 0600); err != nil {
		t.Fatal(err)
	}
	third, err := resolver.ResolveLegacyMetering(context.Background(), binding)
	if err != nil || third.Key[0] != 's' {
		t.Fatal("implicit live key reload", err)
	}
	mutations := []LegacyMeteringBinding{binding, binding, binding, binding, binding}
	mutations[0].ServiceID = "other"
	mutations[1].VersionID = "other"
	mutations[2].Endpoint += "/"
	mutations[3].ToolID = "other"
	mutations[4].RemoteDigest = strings.Repeat("b", 64)
	for _, changed := range mutations {
		if _, err := resolver.ResolveLegacyMetering(context.Background(), changed); err == nil {
			t.Fatal("cross-binding resolution", changed)
		}
	}
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	if _, err := resolver.ResolveLegacyMetering(ctx, binding); err != context.Canceled {
		t.Fatal(err)
	}
	var wg sync.WaitGroup
	for i := 0; i < 8; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			cfg, err := resolver.ResolveLegacyMetering(context.Background(), binding)
			if err == nil {
				cfg.Key[0] = 0
				cfg.Metrics[0] = "changed"
			}
		}()
	}
	wg.Wait()
}
func TestLegacyMeteringRejectsConfigAndKeyCounterexamples(t *testing.T) {
	for _, name := range []string{"unknown-field", "duplicate-binding", "duplicate-adapter", "unknown-metric", "duplicate-metric", "empty-metrics", "missing-binding", "wrong-digest", "private-endpoint", "inline-key", "json-config", "oversize-config", "relative-key", "relative-config", "public-config", "public-key", "short-key", "oversize-key", "symlink-config", "symlink-key"} {
		t.Run(name, func(t *testing.T) {
			config, key, _, document := legacyMeteringFixture(t)
			switch name {
			case "unknown-field":
				document += "owner = \"request\"\n"
			case "duplicate-binding":
				document += "\n" + strings.Replace(document, `adapter_id = "collector"`, `adapter_id = "other"`, 1)
			case "duplicate-adapter":
				document += "\n" + strings.Replace(document, `tool_id = "tool"`, `tool_id = "other"`, 1)
			case "unknown-metric":
				document = strings.Replace(document, `"images"`, `"cash"`, 1)
			case "duplicate-metric":
				document = strings.Replace(document, `"images"`, `"input_tokens"`, 1)
			case "empty-metrics":
				document = strings.Replace(document, `["input_tokens", "images"]`, `[]`, 1)
			case "missing-binding":
				document = strings.Replace(document, `service_id = "service"`+"\n", "", 1)
			case "wrong-digest":
				document = strings.Replace(document, strings.Repeat("a", 64), strings.Repeat("z", 64), 1)
			case "private-endpoint":
				document = strings.Replace(document, "https://example.com/mcp", "https://127.0.0.1/mcp", 1)
			case "inline-key":
				document += "key = \"never-in-config\"\n"
			case "json-config":
				document = `{"adapters":[]}`
			case "oversize-config":
				document += strings.Repeat("# padded\n", 9000)
			case "relative-key":
				document = strings.Replace(document, key, "collector.key", 1)
			case "relative-config":
				config = "metering.toml"
			case "public-key":
				if err := os.Chmod(key, 0644); err != nil {
					t.Fatal(err)
				}
			case "short-key":
				if err := os.WriteFile(key, []byte(strings.Repeat("x", 31)), 0600); err != nil {
					t.Fatal(err)
				}
			case "oversize-key":
				if err := os.WriteFile(key, []byte(strings.Repeat("x", 257)), 0600); err != nil {
					t.Fatal(err)
				}
			case "symlink-key":
				link := key + ".link"
				if err := os.Symlink(key, link); err != nil {
					t.Fatal(err)
				}
				document = strings.Replace(document, key, link, 1)
			}
			if name != "relative-config" {
				if err := os.WriteFile(config, []byte(document), 0600); err != nil {
					t.Fatal(err)
				}
			}
			if name == "public-config" {
				if err := os.Chmod(config, 0644); err != nil {
					t.Fatal(err)
				}
			}
			if name == "symlink-config" {
				link := config + ".link"
				if err := os.Symlink(config, link); err != nil {
					t.Fatal(err)
				}
				config = link
			}
			_, err := LoadLegacyMeteringConfig(config)
			if err == nil {
				t.Fatal("unsafe metering config accepted")
			}
			if strings.Contains(err.Error(), key) || strings.Contains(err.Error(), config) || strings.Contains(err.Error(), "synthetic-key") {
				t.Fatal("error disclosed private material")
			}
		})
	}
}
func TestLegacyMeteringKeySizeEdgesAndDisabledSignatures(t *testing.T) {
	for _, size := range []int{32, 256} {
		config, key, binding, _ := legacyMeteringFixture(t)
		if err := os.WriteFile(key, []byte(strings.Repeat("k", size)), 0400); err != nil {
			t.Fatal(err)
		}
		if err := os.Chmod(key, 0400); err != nil {
			t.Fatal(err)
		}
		resolver, err := LoadLegacyMeteringConfig(config)
		if err != nil {
			t.Fatal(size, err)
		}
		cfg, err := resolver.ResolveLegacyMetering(context.Background(), binding)
		if err != nil || len(cfg.Key) != size {
			t.Fatal(size, err)
		}
	}
	resolver, err := LoadLegacyMeteringConfig("")
	if err != nil || resolver != nil {
		t.Fatal("disabled config", resolver, err)
	}
	data := json.RawMessage(`{"usage":{"input_tokens":3},"_meta":{"lmm_metering":{"adapter_id":"unconfigured","quantities":{"input_tokens":3}}}}`)
	if _, err := ReadLegacyUsage(legacyUsageTestPrice(), data, nil); err == nil {
		t.Fatal("signed report downgraded when configuration absent")
	}
}
