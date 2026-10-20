// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
// Compiled service/tool identities follow fixed main 3948a0a's RFC 4122 UUIDv5
// names. The new exact schemas produce new immutable versions and fresh grants.
package toolmarket

import (
	"crypto/sha256"
	_ "embed"
	"encoding/hex"
	"encoding/json"
)

//go:embed builtin_catalog.json
var builtinCatalogJSON []byte

type CompiledBuiltinTool struct {
	WebTool
	InputSchemaDigest string `json:"input_schema_digest"`
}
type CompiledBuiltinDefinition struct {
	Key           string                `json:"key"`
	Name          string                `json:"name"`
	Description   string                `json:"description"`
	ServiceID     string                `json:"service_id"`
	VersionID     string                `json:"version_id"`
	SchemaVersion string                `json:"schema_version"`
	Digest        string                `json:"digest"`
	Tools         []CompiledBuiltinTool `json:"tools"`
}

// CompiledBuiltinDefinitions returns independent trees. Callers cannot mutate
// the trusted registry used for dispatch or the installer binding manifest.
func CompiledBuiltinDefinitions() []CompiledBuiltinDefinition {
	var result []CompiledBuiltinDefinition
	if err := json.Unmarshal(builtinCatalogJSON, &result); err != nil {
		panic("invalid compiled builtin catalogue")
	}
	return result
}
func compiledBuiltin(service, name string) (CompiledBuiltinDefinition, CompiledBuiltinTool, error) {
	for _, definition := range CompiledBuiltinDefinitions() {
		if definition.Key != service {
			continue
		}
		for _, tool := range definition.Tools {
			if tool.Name == name {
				return definition, tool, nil
			}
		}
	}
	return CompiledBuiltinDefinition{}, CompiledBuiltinTool{}, ErrMissing
}

// CompiledBuiltinCatalogDigest binds readiness to these exact embedded bytes.
func CompiledBuiltinCatalogDigest() string {
	sum := sha256.Sum256(builtinCatalogJSON)
	return hex.EncodeToString(sum[:])
}
