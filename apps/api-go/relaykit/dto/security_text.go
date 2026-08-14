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
	"sort"
	"strings"
)

// SecurityTextProvider separates model-facing request text from token-counting
// metadata. Token counting intentionally includes tool declarations and other
// structural fields, while the security guardrail should inspect instructions,
// conversation content, and tool arguments without matching binary payloads.
type SecurityTextProvider interface {
	GetSecurityText() string
}

// SecurityTextForRequest returns the text that should be inspected before a
// request is sent upstream. The fallback keeps third-party Request
// implementations covered until they implement SecurityTextProvider.
func SecurityTextForRequest(request Request) string {
	if request == nil {
		return ""
	}
	if provider, ok := request.(SecurityTextProvider); ok {
		return provider.GetSecurityText()
	}
	meta := request.GetTokenCountMeta()
	if meta == nil {
		return ""
	}
	return meta.CombineText
}

// SecurityTextFromJSON extracts string values from a JSON event while skipping
// obvious binary/media fields. It is used for OpenAI Realtime events, whose
// prompt-bearing shape varies by event type and API revision.
func SecurityTextFromJSON(raw []byte) string {
	if len(raw) == 0 {
		return ""
	}
	var value any
	if err := json.Unmarshal(raw, &value); err != nil {
		return ""
	}
	parts := make([]string, 0)
	appendSecurityJSONValue(&parts, "", value)
	return joinSecurityText(parts)
}

// SecurityTextFromRealtimeJSON additionally excludes declared session/response
// tools. Their names, descriptions, and JSON schemas describe application
// capabilities rather than an end-user instruction and otherwise create noisy
// false positives on every session update.
func SecurityTextFromRealtimeJSON(raw []byte) string {
	if len(raw) == 0 {
		return ""
	}
	var value any
	if err := json.Unmarshal(raw, &value); err != nil {
		return ""
	}
	parts := make([]string, 0)
	appendRealtimeSecurityJSONValue(&parts, "", "", value)
	return joinSecurityText(parts)
}

func (r *BaseRequest) GetSecurityText() string {
	return ""
}

func (r *GeneralOpenAIRequest) GetSecurityText() string {
	if r == nil {
		return ""
	}
	parts := make([]string, 0, len(r.Messages)+5)
	appendSecurityValue(&parts, r.Prompt)
	appendSecurityValue(&parts, r.Prefix)
	appendSecurityValue(&parts, r.Suffix)
	appendSecurityValue(&parts, r.Input)
	appendSecurityString(&parts, r.Instruction)
	for index := range r.Messages {
		message := &r.Messages[index]
		for _, content := range message.ParseContent() {
			if content.Type == ContentTypeText {
				appendSecurityString(&parts, content.Text)
			}
		}
		appendSecurityRaw(&parts, message.ToolCalls)
	}
	appendSecurityRaw(&parts, r.FunctionCall)
	return joinSecurityText(parts)
}

func (r *OpenAIResponsesRequest) GetSecurityText() string {
	if r == nil {
		return ""
	}
	parts := make([]string, 0, 3)
	appendSecurityRaw(&parts, r.Instructions)
	appendSecurityRaw(&parts, r.Input)
	appendSecurityRaw(&parts, r.Prompt)
	return joinSecurityText(parts)
}

func (r *OpenAIResponsesCompactionRequest) GetSecurityText() string {
	if r == nil {
		return ""
	}
	parts := make([]string, 0, 2)
	appendSecurityRaw(&parts, r.Instructions)
	appendSecurityRaw(&parts, r.Input)
	return joinSecurityText(parts)
}

func (r *ClaudeRequest) GetSecurityText() string {
	if r == nil {
		return ""
	}
	parts := make([]string, 0, len(r.Messages)+2)
	appendSecurityString(&parts, r.Prompt)
	if r.IsStringSystem() {
		appendSecurityString(&parts, r.GetStringSystem())
	} else {
		for _, media := range r.ParseSystem() {
			appendClaudeMediaSecurityText(&parts, media)
		}
	}
	for index := range r.Messages {
		message := &r.Messages[index]
		if message.IsStringContent() {
			appendSecurityString(&parts, message.GetStringContent())
			continue
		}
		content, _ := message.ParseContent()
		for _, media := range content {
			appendClaudeMediaSecurityText(&parts, media)
		}
	}
	return joinSecurityText(parts)
}

func (r *GeminiChatRequest) GetSecurityText() string {
	if r == nil {
		return ""
	}
	parts := make([]string, 0)
	if r.SystemInstructions != nil {
		appendGeminiContentSecurityText(&parts, *r.SystemInstructions)
	}
	for _, content := range r.Contents {
		appendGeminiContentSecurityText(&parts, content)
	}
	for index := range r.Requests {
		appendSecurityString(&parts, r.Requests[index].GetSecurityText())
	}
	return joinSecurityText(parts)
}

func (r *GeminiEmbeddingRequest) GetSecurityText() string {
	if r == nil {
		return ""
	}
	parts := make([]string, 0)
	appendGeminiContentSecurityText(&parts, r.Content)
	return joinSecurityText(parts)
}

func (r *GeminiBatchEmbeddingRequest) GetSecurityText() string {
	if r == nil {
		return ""
	}
	parts := make([]string, 0, len(r.Requests))
	for _, request := range r.Requests {
		if request != nil {
			appendSecurityString(&parts, request.GetSecurityText())
		}
	}
	return joinSecurityText(parts)
}

func (r *ImageRequest) GetSecurityText() string {
	if r == nil {
		return ""
	}
	return strings.TrimSpace(r.Prompt)
}

func (r *AudioRequest) GetSecurityText() string {
	if r == nil {
		return ""
	}
	parts := []string{r.Input, r.Instructions}
	appendSecurityRaw(&parts, r.RefText)
	return joinSecurityText(parts)
}

func (r *EmbeddingRequest) GetSecurityText() string {
	if r == nil {
		return ""
	}
	parts := make([]string, 0)
	appendSecurityValue(&parts, r.Input)
	return joinSecurityText(parts)
}

func (r *RerankRequest) GetSecurityText() string {
	if r == nil {
		return ""
	}
	parts := make([]string, 0, len(r.Documents)+1)
	appendSecurityString(&parts, r.Query)
	for _, document := range r.Documents {
		appendSecurityValue(&parts, document)
	}
	return joinSecurityText(parts)
}

func (r *AlphaSearchRequest) GetSecurityText() string {
	if r == nil {
		return ""
	}
	return SecurityTextFromJSON(r.RawBody)
}

func appendClaudeMediaSecurityText(parts *[]string, media ClaudeMediaMessage) {
	switch media.Type {
	case "text":
		appendSecurityString(parts, media.GetText())
	case "tool_use":
		appendSecurityValue(parts, media.Input)
	case "tool_result":
		appendSecurityValue(parts, media.Content)
	}
}

func appendGeminiContentSecurityText(parts *[]string, content GeminiChatContent) {
	for _, part := range content.Parts {
		appendSecurityString(parts, part.Text)
		if part.FunctionCall != nil {
			appendSecurityValue(parts, part.FunctionCall.Arguments)
		}
		if part.FunctionResponse != nil {
			appendSecurityValue(parts, part.FunctionResponse.Response)
			appendSecurityRaw(parts, part.FunctionResponse.Parts)
		}
		if part.ExecutableCode != nil {
			appendSecurityString(parts, part.ExecutableCode.Code)
		}
		if part.CodeExecutionResult != nil {
			appendSecurityString(parts, part.CodeExecutionResult.Output)
		}
	}
}

func appendSecurityRaw(parts *[]string, raw json.RawMessage) {
	if len(raw) == 0 {
		return
	}
	var value any
	if err := json.Unmarshal(raw, &value); err != nil {
		return
	}
	appendSecurityJSONValue(parts, "", value)
}

func appendSecurityValue(parts *[]string, value any) {
	switch typed := value.(type) {
	case nil:
		return
	case string:
		appendSecurityString(parts, typed)
	case *string:
		if typed != nil {
			appendSecurityString(parts, *typed)
		}
	case json.RawMessage:
		appendSecurityRaw(parts, typed)
	case []string:
		for _, item := range typed {
			appendSecurityString(parts, item)
		}
	case []any:
		for _, item := range typed {
			appendSecurityValue(parts, item)
		}
	case map[string]any:
		appendSecurityJSONValue(parts, "", typed)
	}
}

func appendSecurityJSONValue(parts *[]string, field string, value any) {
	switch typed := value.(type) {
	case string:
		if shouldSkipSecurityField(field, typed) {
			return
		}
		appendSecurityString(parts, typed)
	case []any:
		for _, item := range typed {
			appendSecurityJSONValue(parts, field, item)
		}
	case map[string]any:
		keys := sortedSecurityJSONKeys(typed)
		for _, key := range keys {
			appendSecurityJSONValue(parts, key, typed[key])
		}
	}
}

func appendRealtimeSecurityJSONValue(parts *[]string, container, field string, value any) {
	switch typed := value.(type) {
	case string:
		if shouldSkipSecurityField(field, typed) {
			return
		}
		appendSecurityString(parts, typed)
	case []any:
		for _, item := range typed {
			appendRealtimeSecurityJSONValue(parts, container, field, item)
		}
	case map[string]any:
		keys := sortedSecurityJSONKeys(typed)
		for _, key := range keys {
			normalizedKey := strings.ToLower(strings.TrimSpace(key))
			if normalizedKey == "tools" && (container == "session" || container == "response") {
				continue
			}
			appendRealtimeSecurityJSONValue(parts, normalizedKey, normalizedKey, typed[key])
		}
	}
}

func sortedSecurityJSONKeys(value map[string]any) []string {
	keys := make([]string, 0, len(value))
	for key := range value {
		keys = append(keys, key)
	}
	sort.Strings(keys)
	return keys
}

func shouldSkipSecurityField(field string, value string) bool {
	field = strings.ToLower(strings.TrimSpace(field))
	if field == "audio" || field == "input_audio" || field == "b64_json" || strings.Contains(field, "base64") {
		return true
	}
	if field == "image_url" || field == "file_url" || field == "file_uri" || field == "fileuri" {
		return true
	}
	if (field == "data" || field == "image" || field == "inline_data" || field == "inlinedata") &&
		(len(value) > 512 || strings.HasPrefix(value, "data:")) {
		return true
	}
	return false
}

func appendSecurityString(parts *[]string, value string) {
	value = strings.TrimSpace(value)
	if value != "" {
		*parts = append(*parts, value)
	}
}

func joinSecurityText(parts []string) string {
	filtered := make([]string, 0, len(parts))
	for _, part := range parts {
		appendSecurityString(&filtered, part)
	}
	return strings.Join(filtered, "\n")
}
