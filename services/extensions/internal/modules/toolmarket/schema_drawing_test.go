// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package toolmarket

import (
	"encoding/json"
	"strings"
	"testing"
)

func TestCompiledDrawingSchemaActuallyEnforcesReferenceConstraints(t *testing.T) {
	_, tool, err := compiledBuiltin("drawing", "drawing.generate")
	if err != nil {
		t.Fatal(err)
	}
	for _, name := range []string{"prompt only", "valid references", "sha uppercase", "sha short", "sha long", "sha alphabet", "empty references", "too many", "wrong media", "zero bytes", "too many bytes", "unknown metadata"} {
		t.Run(name, func(t *testing.T) {
			image := map[string]any{"media_type": "image/png", "sha256": strings.Repeat("a", 64), "byte_length": 100}
			args := map[string]any{"prompt": "test"}
			refs := []any{image}
			switch name {
			case "sha uppercase":
				image["sha256"] = strings.Repeat("A", 64)
			case "sha short":
				image["sha256"] = strings.Repeat("a", 63)
			case "sha long":
				image["sha256"] = strings.Repeat("a", 65)
			case "sha alphabet":
				image["sha256"] = strings.Repeat("g", 64)
			case "empty references":
				refs = []any{}
			case "too many":
				for len(refs) < 9 {
					refs = append(refs, image)
				}
			case "wrong media":
				image["media_type"] = "image/gif"
			case "zero bytes":
				image["byte_length"] = 0
			case "too many bytes":
				image["byte_length"] = DrawingUploadFileLimit + 1
			case "unknown metadata":
				image["url"] = "https://example.invalid/image"
			}
			if name != "prompt only" {
				args["reference_images"] = refs
			}
			raw, _ := json.Marshal(args)
			err := validateArgs(json.RawMessage(tool.InputSchema), raw)
			valid := name == "prompt only" || name == "valid references"
			if (err == nil) != valid {
				t.Fatal("compiled schema constraint was ignored or valid schema rejected", err)
			}
		})
	}
}

func TestNativeSchemaPatternSubsetIsFixedAndBounded(t *testing.T) {
	for _, pattern := range []string{"", "a+", "^(a+)+$", "(?=x)", strings.Repeat("a", 1024)} {
		raw, _ := json.Marshal(map[string]any{"type": "object", "properties": map[string]any{"value": map[string]any{"type": "string", "pattern": pattern}}})
		if _, err := readSchema(raw, 0); err == nil {
			t.Fatal("unreviewed expression accepted", pattern)
		}
	}
	if _, err := readSchema(json.RawMessage(`{"type":"integer","pattern":"^[0-9a-f]{64}$"}`), 0); err == nil {
		t.Fatal("string constraint on integer ignored")
	}
}
