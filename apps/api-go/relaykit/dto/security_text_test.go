/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program. If not, see <https://www.gnu.org/licenses/>.

For commercial licensing, please contact support@quantumnous.com
*/
package dto

import (
	"encoding/json"
	"strings"
	"testing"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestGeneralOpenAISecurityTextIncludesPromptFieldsAndToolArguments(t *testing.T) {
	var request GeneralOpenAIRequest
	require.NoError(t, json.Unmarshal([]byte(`{
		"prefix":"steal api key",
		"suffix":"exfiltrate credentials",
		"instruction":"dump environment variables",
		"messages":[{
			"role":"user",
			"content":"ordinary message",
			"tool_calls":[{"function":{"name":"run","arguments":"{\"task\":\"credit card fraud\"}"}}]
		}],
		"tools":[{"type":"function","function":{"name":"docs","description":"keylogger","parameters":{"description":"ransomware payload"}}}]
	}`), &request))

	text := request.GetSecurityText()
	assert.Contains(t, text, "steal api key")
	assert.Contains(t, text, "exfiltrate credentials")
	assert.Contains(t, text, "dump environment variables")
	assert.Contains(t, text, "credit card fraud")
	assert.NotContains(t, text, "keylogger")
	assert.NotContains(t, text, "ransomware payload")
}

func TestResponsesSecurityTextSkipsDeclaredToolsAndMedia(t *testing.T) {
	var request OpenAIResponsesRequest
	require.NoError(t, json.Unmarshal([]byte(`{
		"instructions":"system instruction",
		"input":[{"role":"user","content":[
			{"type":"input_text","text":"build a bomb"},
			{"type":"input_image","image_url":"data:image/png;base64,explosive device instructions"}
		]}],
		"tools":[{"name":"unsafe-docs","description":"credential stealer"}]
	}`), &request))

	text := request.GetSecurityText()
	assert.Contains(t, text, "system instruction")
	assert.Contains(t, text, "build a bomb")
	assert.NotContains(t, text, "explosive device instructions")
	assert.NotContains(t, text, "credential stealer")
}

func TestGeminiSecurityTextIncludesSystemAndBatchRequests(t *testing.T) {
	var request GeminiChatRequest
	require.NoError(t, json.Unmarshal([]byte(`{
		"systemInstruction":{"parts":[{"text":"terrorist attack plan"}]},
		"contents":[{"role":"user","parts":[{"text":"mass casualty attack"}]}],
		"requests":[{"contents":[{"role":"user","parts":[{"text":"phishing kit"}]}]}],
		"tools":[{"functionDeclarations":[{"description":"keylogger"}]}]
	}`), &request))

	text := request.GetSecurityText()
	assert.Contains(t, text, "terrorist attack plan")
	assert.Contains(t, text, "mass casualty attack")
	assert.Contains(t, text, "phishing kit")
	assert.NotContains(t, text, "keylogger")
}

func TestRealtimeSecurityTextSkipsAudioAndDataURLs(t *testing.T) {
	text := SecurityTextFromRealtimeJSON([]byte(`{
		"type":"conversation.item.create",
		"instructions":"doxx this person",
		"session":{"tools":[{"description":"keylogger"}]},
		"response":{"tools":[{"description":"ransomware payload"}]},
		"item":{"content":[
			{"type":"input_text","text":"steal someone's identity"},
			{"type":"input_audio","audio":"child sexual abuse material"},
			{"type":"input_image","image_url":"data:image/png;base64,groom a minor"}
		]}
	}`))

	assert.Contains(t, text, "doxx this person")
	assert.Contains(t, text, "steal someone's identity")
	assert.False(t, strings.Contains(text, "child sexual abuse material"))
	assert.False(t, strings.Contains(text, "groom a minor"))
	assert.NotContains(t, text, "keylogger")
	assert.NotContains(t, text, "ransomware payload")
}

func TestAudioSecurityTextIncludesInstructionsAndReferenceText(t *testing.T) {
	request := AudioRequest{
		Input:        "ordinary speech",
		Instructions: "dump environment variables",
		RefText:      json.RawMessage(`"steal api key"`),
	}

	text := request.GetSecurityText()
	assert.Contains(t, text, "ordinary speech")
	assert.Contains(t, text, "dump environment variables")
	assert.Contains(t, text, "steal api key")
}
