// Copyright (c) 2025-2026 QuantumNous. All rights reserved.

package agent

import (
	"encoding/json"
	"testing"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestTextNormalizesCompatibleContentShapes(t *testing.T) {
	tests := []struct {
		name string
		raw  string
		want string
	}{
		{name: "string", raw: `"hello"`, want: "hello"},
		{name: "string list", raw: `["hello"," ","world"]`, want: "hello world"},
		{name: "typed parts", raw: `[{"type":"output_text","text":"hello"},{"type":"image","text":"hidden"}]`, want: "hello"},
		{name: "null", raw: `null`, want: ""},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			assert.Equal(t, test.want, Text(json.RawMessage(test.raw)))
		})
	}
}

func TestParseAndBytes(t *testing.T) {
	response, err := Parse([]byte(`{"choices":[{"message":{"role":"assistant","content":"ok"}}]}`))
	require.NoError(t, err)
	require.Len(t, response.Choices, 1)
	assert.Equal(t, "ok", Text(response.Choices[0].Message.Content))

	messages := []Message{{
		Role: "assistant", Content: "answer", Name: "guide", ToolCallID: "result",
		ToolCalls: []Call{{ID: "call", Type: "function", Function: CallFunction{Name: "lookup", Arguments: `{}`}}},
	}}
	assert.Equal(t, len("assistantanswerguideresultcallfunctionlookup{}"), Bytes(messages))
}

func TestParseRejectsEmptyOrInvalidResponses(t *testing.T) {
	_, err := Parse(nil)
	assert.Error(t, err)
	_, err = Parse([]byte(`{`))
	assert.Error(t, err)
}
