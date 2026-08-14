package relayconvert

import (
	"context"
	"github.com/LIghtJUNction/api.lmm.best/relaykit/dto"
	"github.com/LIghtJUNction/api.lmm.best/relaykit/relayconvert/convmeta"
	claudemessages "github.com/LIghtJUNction/api.lmm.best/relaykit/relayconvert/internal/claude_messages"
	geminichat "github.com/LIghtJUNction/api.lmm.best/relaykit/relayconvert/internal/gemini_chat"
	oaichat "github.com/LIghtJUNction/api.lmm.best/relaykit/relayconvert/internal/oai_chat"
	oairesponses "github.com/LIghtJUNction/api.lmm.best/relaykit/relayconvert/internal/oai_responses"
	sharedgemini "github.com/LIghtJUNction/api.lmm.best/relaykit/relayconvert/internal/shared/gemini"
)

func ClaudeMessagesRequestToOpenAIChat(claudeRequest dto.ClaudeRequest, info convmeta.Meta) (*dto.GeneralOpenAIRequest, error) {
	return claudemessages.ClaudeMessagesRequestToOpenAIChat(claudeRequest, info)
}

func OpenAIChatRequestToClaudeMessages(c context.Context, info convmeta.Meta, textRequest dto.GeneralOpenAIRequest) (*dto.ClaudeRequest, error) {
	return oaichat.OpenAIChatRequestToClaudeMessages(c, info, textRequest)
}

func GeminiGenerateContentRequestToOpenAIChat(geminiRequest *dto.GeminiChatRequest, info convmeta.Meta) (*dto.GeneralOpenAIRequest, error) {
	return geminichat.GeminiGenerateContentRequestToOpenAIChat(geminiRequest, info)
}

func OpenAIChatRequestToGeminiGenerateContent(c context.Context, textRequest dto.GeneralOpenAIRequest, info convmeta.Meta) (*dto.GeminiChatRequest, error) {
	return oaichat.OpenAIChatRequestToGeminiGenerateContent(c, textRequest, info)
}

func ApplyGeminiThinkingConfig(geminiRequest *dto.GeminiChatRequest, info convmeta.Meta, oaiRequest ...dto.GeneralOpenAIRequest) {
	sharedgemini.ApplyThinkingConfig(geminiRequest, info, oaiRequest...)
}

func ChatCompletionsRequestToResponsesRequest(req *dto.GeneralOpenAIRequest) (*dto.OpenAIResponsesRequest, error) {
	return oaichat.ChatCompletionsRequestToResponsesRequest(req)
}

func ResponsesRequestToChatCompletionsRequest(req *dto.OpenAIResponsesRequest) (*dto.GeneralOpenAIRequest, error) {
	return oairesponses.ResponsesRequestToChatCompletionsRequest(req)
}

func OpenAIResponsesRequestToClaudeMessages(c context.Context, info convmeta.Meta, req *dto.OpenAIResponsesRequest) (*dto.ClaudeRequest, error) {
	return oairesponses.OpenAIResponsesRequestToClaudeMessages(c, info, req)
}

func OpenAIResponsesRequestToGeminiChat(c context.Context, req *dto.OpenAIResponsesRequest, info convmeta.Meta) (*dto.GeminiChatRequest, error) {
	return oairesponses.OpenAIResponsesRequestToGeminiChat(c, req, info)
}
