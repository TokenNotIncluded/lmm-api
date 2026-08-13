package claudemessages

import (
	"encoding/json"
	"testing"

	"github.com/QuantumNous/new-api/relaykit/dto"
	"github.com/QuantumNous/new-api/relaykit/relayconvert/convmeta"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestClaudeMessagesRequestToOpenAIChatMapsEffortInCompatibilityMode(t *testing.T) {
	budget := 512
	claudeRequest := dto.ClaudeRequest{
		Model:        "gpt-4o-mini",
		OutputConfig: json.RawMessage(`{"effort":"high"}`),
		Thinking: &dto.Thinking{
			Type:         "enabled",
			BudgetTokens: &budget,
		},
	}

	info := &convmeta.Values{
		Options: &convmeta.Options{
			EnableMessagesToGPTCompatibility: true,
		},
	}

	openAIRequest, err := ClaudeMessagesRequestToOpenAIChat(claudeRequest, info)
	require.NoError(t, err)
	assert.Equal(t, "high", openAIRequest.ReasoningEffort)
	assert.True(t, len(openAIRequest.Reasoning) == 0)
}

func TestClaudeMessagesRequestToOpenAIChatMapsThinkingToEffortInCompatibilityMode(t *testing.T) {
	claudeRequest := dto.ClaudeRequest{
		Model: "gpt-4o-mini",
		Thinking: &dto.Thinking{
			Type:         "enabled",
			BudgetTokens: intPtr(4096),
		},
	}

	info := &convmeta.Values{
		Options: &convmeta.Options{
			EnableMessagesToGPTCompatibility: true,
		},
	}

	openAIRequest, err := ClaudeMessagesRequestToOpenAIChat(claudeRequest, info)
	require.NoError(t, err)
	assert.Equal(t, "high", openAIRequest.ReasoningEffort)
}

func TestClaudeMessagesRequestToOpenAIChatDoesNotMapEffortForNonGPTCompatibilityModels(t *testing.T) {
	claudeRequest := dto.ClaudeRequest{
		Model:        "claude-3-7-sonnet-20240229",
		OutputConfig: json.RawMessage(`{"effort":"high"}`),
	}

	info := &convmeta.Values{
		Options: &convmeta.Options{
			EnableMessagesToGPTCompatibility: false,
		},
	}

	openAIRequest, err := ClaudeMessagesRequestToOpenAIChat(claudeRequest, info)
	require.NoError(t, err)
	assert.Empty(t, openAIRequest.ReasoningEffort)
}

func TestClaudeMessagesRequestToOpenAIChatKeepsTextAndToolUseTogether(t *testing.T) {
	assistantContent := []dto.ClaudeMediaMessage{
		{
			Type: "text",
			Text: stringPtr("look up"),
		},
		{
			Type:         "tool_use",
			Id:           "tool_1",
			Name:         "lookup",
			Input:        map[string]interface{}{"q": "alpha"},
			CacheControl: nil,
		},
	}
	claudeRequest := dto.ClaudeRequest{
		Model: "gpt-4o-mini",
		Messages: []dto.ClaudeMessage{
			{
				Role:    "assistant",
				Content: assistantContent,
			},
		},
	}

	info := &convmeta.Values{
		Options: &convmeta.Options{
			EnableMessagesToGPTCompatibility: true,
		},
	}

	openAIRequest, err := ClaudeMessagesRequestToOpenAIChat(claudeRequest, info)
	require.NoError(t, err)
	require.Len(t, openAIRequest.Messages, 1)

	assistantMessage := openAIRequest.Messages[0]
	assert.Equal(t, "assistant", assistantMessage.Role)

	mediaContent := assistantMessage.ParseContent()
	require.Len(t, mediaContent, 1)
	assert.Equal(t, "text", mediaContent[0].Type)
	assert.Equal(t, "look up", mediaContent[0].Text)

	toolCalls := assistantMessage.ParseToolCalls()
	require.Len(t, toolCalls, 1)
	assert.Equal(t, "function", toolCalls[0].Type)
	assert.Equal(t, "lookup", toolCalls[0].Function.Name)
}

func TestClaudeMessagesRequestToOpenAIChatDropsThinkingHistoryBlocks(t *testing.T) {
	claudeRequest := dto.ClaudeRequest{
		Model:  "gpt-4o-mini",
		System: "sys prompt",
		Messages: []dto.ClaudeMessage{
			{
				Role: "assistant",
				Content: []dto.ClaudeMediaMessage{
					{
						Type:         "thinking",
						Text:         stringPtr("hidden reasoning"),
						CacheControl: json.RawMessage(`null`),
					},
					{
						Type:         "redacted_thinking",
						Text:         stringPtr("redacted"),
						CacheControl: json.RawMessage(`null`),
					},
				},
			},
		},
	}

	info := &convmeta.Values{
		Options: &convmeta.Options{
			EnableMessagesToGPTCompatibility: true,
		},
	}

	openAIRequest, err := ClaudeMessagesRequestToOpenAIChat(claudeRequest, info)
	require.NoError(t, err)
	require.Len(t, openAIRequest.Messages, 1)
	assert.Equal(t, "system", openAIRequest.Messages[0].Role)
}

func intPtr(v int) *int {
	return &v
}

func stringPtr(v string) *string {
	return &v
}
