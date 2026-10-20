// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package toolmarket

import (
	"encoding/json"
	"testing"
)

func TestLegacySchemaExactLargeNumbersAndLocalReferences(t *testing.T) {
	schema := json.RawMessage(`{"type":"object","$defs":{"number":{"type":"integer","maximum":9007199254740993}},"properties":{"n":{"$ref":"#/$defs/number"},"kind":{"oneOf":[{"const":"image"},{"const":"text"}]}},"required":["n","kind"],"additionalProperties":false}`)
	good := json.RawMessage(`{"n":9007199254740993,"kind":"image"}`)
	args, err := ValidateLegacyArguments(schema, good)
	if err != nil || args["n"].(json.Number).String() != "9007199254740993" {
		t.Fatal("exact numeric validation changed", err)
	}
	for _, bad := range []string{`{"n":9007199254740994,"kind":"image"}`, `{"n":1,"kind":"unknown"}`, `{"n":1,"n":2,"kind":"text"}`, `{"n":1e999999,"kind":"text"}`} {
		if _, err := ValidateLegacyArguments(schema, json.RawMessage(bad)); err == nil {
			t.Fatal("invalid or ambiguous arguments accepted")
		}
	}
}
func TestLegacySchemaRejectsRemoteLoadersAndNumericAmplification(t *testing.T) {
	for _, schema := range []string{`{"type":"object","properties":{"x":{"$ref":"https://attacker.invalid/schema"}}}`, `{"type":"object","$id":"https://attacker.invalid/schema"}`, `{"type":"object","$dynamicRef":"#x"}`, `{"type":"object","properties":{"n":{"maximum":1e999999}}}`, `{"type":"object","$schema":"https://attacker.invalid/meta"}`} {
		if ValidateLegacySchema(json.RawMessage(schema)) == nil {
			t.Fatal("external or unbounded schema accepted")
		}
	}
	schema := json.RawMessage(`{"$schema":"http://json-schema.org/draft-07/schema#","type":"object","properties":{"n":{"type":"integer","minimum":2}},"required":["n"]}`)
	if ValidateLegacyDocument(schema, json.RawMessage(`{"n":2}`)) != nil || ValidateLegacyDocument(schema, json.RawMessage(`{"n":1}`)) == nil {
		t.Fatal("draft7 output constraint ignored")
	}
}
