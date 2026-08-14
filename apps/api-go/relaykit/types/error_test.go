package types

import (
	"errors"
	"net/http"
	"testing"

	"github.com/stretchr/testify/require"
)

func TestToOpenAIErrorUsesActionableFallbackForEmptyUpstreamMessage(t *testing.T) {
	err := InitOpenAIError(ErrorCodeDoRequestFailed, http.StatusBadGateway)

	openAIError := err.ToOpenAIError()

	require.Equal(t, "upstream request failed (HTTP 502); please retry.", openAIError.Message)
	require.Equal(t, ErrorCodeDoRequestFailed, openAIError.Code)
}

func TestToOpenAIErrorPreservesUnderlyingMessage(t *testing.T) {
	err := NewOpenAIError(errors.New("upstream returned a useful message"), ErrorCodeBadResponse, http.StatusBadGateway)

	openAIError := err.ToOpenAIError()

	require.Equal(t, "upstream returned a useful message", openAIError.Message)
}

func TestToOpenAIErrorMasksUsefulFallbackMessage(t *testing.T) {
	err := InitOpenAIError(ErrorCodeDoRequestFailed, http.StatusBadGateway)
	err.Err = errors.New("upstream rejected api_key:secret-value")

	openAIError := err.ToOpenAIError()

	require.Equal(t, "upstream rejected api_key:***", openAIError.Message)
}

func TestAdvancedSecurityErrorKeepsStableCodeForClaude(t *testing.T) {
	err := NewErrorWithStatusCode(
		errors.New("prompt blocked by advanced security guardrail"),
		ErrorCodeAdvancedSecurity,
		http.StatusBadRequest,
	)

	claudeError := err.ToClaudeError()

	require.Equal(t, "invalid_request_error", claudeError.Type)
	require.Equal(t, string(ErrorCodeAdvancedSecurity), claudeError.Code)
	require.Contains(t, claudeError.Message, "prompt blocked")
}

func TestAdvancedSecurityErrorUsesNativeGeminiEnvelope(t *testing.T) {
	err := NewErrorWithStatusCode(
		errors.New("prompt blocked by advanced security guardrail"),
		ErrorCodeAdvancedSecurity,
		http.StatusBadRequest,
	)

	geminiError := err.ToGeminiError()

	require.Equal(t, http.StatusBadRequest, geminiError.Error.Code)
	require.Equal(t, "INVALID_ARGUMENT", geminiError.Error.Status)
	require.Contains(t, geminiError.Error.Message, "prompt blocked")
	require.Len(t, geminiError.Error.Details, 1)
	require.Equal(t, string(ErrorCodeAdvancedSecurity), geminiError.Error.Details[0].Reason)
}
