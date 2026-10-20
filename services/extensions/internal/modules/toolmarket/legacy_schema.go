// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
// Validation semantics follow fixed main 3948a0a service/tool_market_remote.go
// and tool_market_numbers.go; the maintained validator is Apache-2.0 licensed.
package toolmarket

import (
	"encoding/json"
	"strconv"
	"strings"

	"github.com/santhosh-tekuri/jsonschema/v6"
)

type legacyNoSchemaLoader struct{}

func (legacyNoSchemaLoader) Load(string) (any, error) { return nil, ErrInvalid }
func legacyExactBounded(value any, depth int) bool {
	if depth > 32 {
		return false
	}
	switch v := value.(type) {
	case json.Number:
		parts := strings.FieldsFunc(v.String(), func(r rune) bool { return r == 'e' || r == 'E' })
		if len(parts) == 0 || len(parts) > 2 || len(strings.TrimLeft(strings.ReplaceAll(parts[0], ".", ""), "-")) > 256 {
			return false
		}
		if len(parts) == 2 {
			n, err := strconv.ParseInt(parts[1], 10, 32)
			if err != nil || n < -1024 || n > 1024 {
				return false
			}
		}
	case map[string]any:
		for _, child := range v {
			if !legacyExactBounded(child, depth+1) {
				return false
			}
		}
	case []any:
		for _, child := range v {
			if !legacyExactBounded(child, depth+1) {
				return false
			}
		}
	}
	return true
}
func legacySchemaReferencesSafe(value any) bool {
	switch v := value.(type) {
	case map[string]any:
		for key, child := range v {
			if key == "$id" || key == "$dynamicRef" || key == "$dynamicAnchor" {
				return false
			}
			if key == "$ref" {
				ref, ok := child.(string)
				if !ok || !strings.HasPrefix(ref, "#/") {
					return false
				}
			}
			if !legacySchemaReferencesSafe(child) {
				return false
			}
		}
	case []any:
		for _, child := range v {
			if !legacySchemaReferencesSafe(child) {
				return false
			}
		}
	}
	return true
}
func legacySchema(raw json.RawMessage) (*jsonschema.Schema, error) {
	if len(raw) == 0 || len(raw) > 16384 {
		return nil, ErrInvalid
	}
	var tree any
	if decodeStrict(raw, &tree) != nil || !legacyExactBounded(tree, 0) || !legacySchemaReferencesSafe(tree) {
		return nil, ErrInvalid
	}
	schema, ok := tree.(map[string]any)
	if !ok || schema["type"] != "object" {
		return nil, ErrInvalid
	}
	switch schema["$schema"] {
	case nil, "", "http://json-schema.org/draft-07/schema#", "https://json-schema.org/draft-07/schema#", "https://json-schema.org/draft/2020-12/schema":
	default:
		return nil, ErrInvalid
	}
	if schema["$schema"] == "" {
		delete(schema, "$schema")
	}
	compiler := jsonschema.NewCompiler()
	compiler.DefaultDraft(jsonschema.Draft2020)
	compiler.UseLoader(legacyNoSchemaLoader{})
	const endpoint = "https://lmm.invalid/tool-market/schema"
	if compiler.AddResource(endpoint, schema) != nil {
		return nil, ErrInvalid
	}
	result, err := compiler.Compile(endpoint)
	if err != nil {
		return nil, ErrInvalid
	}
	return result, nil
}
func ValidateLegacySchema(raw json.RawMessage) error { _, err := legacySchema(raw); return err }
func ValidateLegacyArguments(schema, raw json.RawMessage) (map[string]any, error) {
	compiled, err := legacySchema(schema)
	if err != nil {
		return nil, err
	}
	var arguments map[string]any
	if len(raw) == 0 || len(raw) > 128<<10 || decodeStrict(raw, &arguments) != nil || arguments == nil || !legacyExactBounded(arguments, 0) || compiled.Validate(arguments) != nil {
		return nil, ErrInvalid
	}
	return arguments, nil
}
func ValidateLegacyDocument(schema, raw json.RawMessage) error {
	compiled, err := legacySchema(schema)
	if err != nil {
		return err
	}
	var document any
	if len(raw) > 2<<20 || strictLegacyJSON(raw, &document, false) != nil || !legacyExactBounded(document, 0) || compiled.Validate(document) != nil {
		return ErrInvalid
	}
	return nil
}
