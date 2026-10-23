// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package toolmarket

import (
	"bytes"
	"context"
	"encoding/hex"
	"errors"
	"path/filepath"
	"strings"
	"unicode"
	"unicode/utf8"

	"github.com/BurntSushi/toml"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/coreclient"
)

type legacyMeteringDocument struct {
	Adapters []legacyMeteringAdapterFile `toml:"adapters"`
}
type legacyMeteringAdapterFile struct {
	ServiceID    string   `toml:"service_id"`
	VersionID    string   `toml:"version_id"`
	Endpoint     string   `toml:"endpoint"`
	ToolID       string   `toml:"tool_id"`
	RemoteDigest string   `toml:"remote_digest"`
	AdapterID    string   `toml:"adapter_id"`
	KeyFile      string   `toml:"key_file"`
	Metrics      []string `toml:"metrics"`
}
type legacyFileMeteringResolver struct {
	adapters map[LegacyMeteringBinding]LegacyMeteringConfig
}

// LoadLegacyMeteringConfig loads one operator-owned immutable snapshot. Empty
// path explicitly disables verified metering; a supplied signature must still
// fail closed in ReadLegacyUsage. Neither file paths nor key bytes enter errors.
// Callers set per-call Context only on the independently copied resolved value.
func LoadLegacyMeteringConfig(path string) (LegacyMeteringResolver, error) {
	if path == "" {
		return nil, nil
	}
	invalid := func() (LegacyMeteringResolver, error) {
		return nil, errors.New("invalid private tool metering configuration")
	}
	if !legacyMeteringPrivatePath(path) {
		return invalid()
	}
	raw, err := coreclient.ReadPrivateFile(path, 64<<10)
	if err != nil || !utf8.Valid(raw) {
		return invalid()
	}
	var cfg legacyMeteringDocument
	metadata, err := toml.NewDecoder(bytes.NewReader(raw)).Decode(&cfg)
	if err != nil || len(metadata.Undecoded()) != 0 || len(cfg.Adapters) == 0 || len(cfg.Adapters) > 100 {
		return invalid()
	}
	allowed := map[string]bool{"adapters": true, "adapters.service_id": true, "adapters.version_id": true, "adapters.endpoint": true, "adapters.tool_id": true, "adapters.remote_digest": true, "adapters.adapter_id": true, "adapters.key_file": true, "adapters.metrics": true}
	for _, key := range metadata.Keys() {
		if !allowed[key.String()] {
			return invalid()
		}
	}
	resolver := &legacyFileMeteringResolver{adapters: map[LegacyMeteringBinding]LegacyMeteringConfig{}}
	ids := map[string]bool{}
	for _, adapter := range cfg.Adapters {
		binding := LegacyMeteringBinding{ServiceID: adapter.ServiceID, VersionID: adapter.VersionID, Endpoint: adapter.Endpoint, ToolID: adapter.ToolID, RemoteDigest: adapter.RemoteDigest}
		if !legacyMeteringText(adapter.ServiceID, 128) || !legacyMeteringText(adapter.VersionID, 128) || !legacyMeteringText(adapter.ToolID, 128) || !legacyMeteringText(adapter.AdapterID, 128) || !legacyMeteringPrivatePath(adapter.KeyFile) || len(adapter.RemoteDigest) != 64 || len(adapter.Metrics) == 0 || len(adapter.Metrics) > len(legacyMetricScales) {
			return invalid()
		}
		if _, err := hex.DecodeString(adapter.RemoteDigest); err != nil {
			return invalid()
		}
		if _, err := (&network{}).url(adapter.Endpoint); err != nil {
			return invalid()
		}
		if _, exists := resolver.adapters[binding]; exists || ids[adapter.AdapterID] {
			return invalid()
		}
		metrics := map[string]bool{}
		for _, metric := range adapter.Metrics {
			if _, ok := legacyMetricScales[metric]; !ok || metrics[metric] {
				return invalid()
			}
			metrics[metric] = true
		}
		key, err := coreclient.ReadPrivateFile(adapter.KeyFile, 256)
		if err != nil || len(key) < 32 || len(key) > 256 {
			return invalid()
		}
		ids[adapter.AdapterID] = true
		resolver.adapters[binding] = LegacyMeteringConfig{AdapterID: adapter.AdapterID, Key: append([]byte(nil), key...), Metrics: append([]string(nil), adapter.Metrics...)}
	}
	return resolver, nil
}
func legacyMeteringPrivatePath(path string) bool {
	return len(path) > 0 && len(path) <= 4096 && filepath.IsAbs(path) && filepath.Clean(path) == path && !strings.ContainsAny(path, "\r\n\x00")
}
func legacyMeteringText(text string, max int) bool {
	if text == "" || len(text) > max || !utf8.ValidString(text) || strings.TrimSpace(text) != text {
		return false
	}
	for _, r := range text {
		if unicode.IsControl(r) {
			return false
		}
	}
	return true
}
func (r *legacyFileMeteringResolver) ResolveLegacyMetering(ctx context.Context, binding LegacyMeteringBinding) (*LegacyMeteringConfig, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	if r == nil {
		return nil, ErrDenied
	}
	cfg, ok := r.adapters[binding]
	if !ok {
		return nil, ErrDenied
	}
	copy := cfg
	copy.Key = append([]byte(nil), cfg.Key...)
	copy.Metrics = append([]string(nil), cfg.Metrics...)
	return &copy, nil
}
