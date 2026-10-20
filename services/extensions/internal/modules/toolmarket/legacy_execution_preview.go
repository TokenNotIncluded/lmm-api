// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package toolmarket

import (
	"encoding/json"
	"strings"
)

// LegacyBuiltinPublicPreview rebuilds both MCP representations from a filtered
// summary. Private continuation/proof tokens and unfinished image bytes never
// become public because a domain happened to include them in its text package.
func LegacyBuiltinPublicPreview(r LegacyExecution) json.RawMessage {
	if r.Builtin == nil || len(r.Builtin.PreviewResult) == 0 || len(r.Builtin.PreviewResult) > 2<<20 {
		return nil
	}
	var envelope map[string]any
	if strictLegacyJSON(r.Builtin.PreviewResult, &envelope, false) != nil {
		return nil
	}
	value, ok := envelope["structuredContent"]
	if !ok {
		return nil
	}
	protected := []string{}
	var continuation map[string]any
	if strictLegacyJSON(r.Builtin.Continuation, &continuation, false) == nil {
		for key, v := range continuation {
			if key != "operation_id" {
				if s, ok := v.(string); ok && s != "" {
					protected = append(protected, s)
				}
			}
		}
	}
	var clean func(any, int) any
	clean = func(value any, depth int) any {
		if depth > 32 {
			return nil
		}
		switch v := value.(type) {
		case map[string]any:
			out := map[string]any{}
			for key, child := range v {
				normalized := strings.ToLower(strings.NewReplacer("_", "", "-", "", " ", "").Replace(key))
				switch normalized {
				case "confirmation", "confirmationtoken", "continuation", "requeststate", "token", "accesstoken", "refreshtoken", "secret", "credential", "signature", "output", "b64json":
					continue
				}
				if sensitiveResultField(key) {
					continue
				}
				out[key] = clean(child, depth+1)
			}
			return out
		case []any:
			out := make([]any, len(v))
			for i, child := range v {
				out[i] = clean(child, depth+1)
			}
			return out
		case string:
			for _, secret := range protected {
				v = strings.ReplaceAll(v, secret, "[REDACTED]")
			}
			return v
		default:
			return value
		}
	}
	filtered := clean(value, 0)
	text, e := json.Marshal(filtered)
	if e != nil {
		return nil
	}
	raw, e := json.Marshal(map[string]any{"content": []any{map[string]any{"type": "text", "text": string(text)}}, "structuredContent": filtered, "isError": false})
	if e != nil || len(raw) > 2<<20 {
		return nil
	}
	return raw
}
