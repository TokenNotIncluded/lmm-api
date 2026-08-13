package controller

import (
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/QuantumNous/new-api/common"
	"github.com/gin-gonic/gin"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestAssistantRelayRecorderHasHardByteBudget(t *testing.T) {
	gin.SetMode(gin.TestMode)
	context, _ := gin.CreateTestContext(httptest.NewRecorder())
	recorder := newAssistantRelayRecorder(context.Writer)
	written, err := recorder.WriteString(strings.Repeat("x", assistantUpstreamResponseMaxBytes+1))
	require.NoError(t, err, "the recorder must contain overflow without destabilizing the relay writer")
	assert.Equal(t, assistantUpstreamResponseMaxBytes+1, written)
	assert.ErrorIs(t, recorder.writeErr, common.ErrLimitExceeded)
	assert.LessOrEqual(t, recorder.body.Len(), assistantUpstreamResponseMaxBytes)
}

func TestAssistantContextBudgetIncludesToolArguments(t *testing.T) {
	messages := []assistantOpenAIMessage{{
		Role: "assistant",
		ToolCalls: []assistantOpenAIToolCall{{
			ID: "call", Type: "function",
			Function: assistantOpenAIToolCallFunction{Name: "tool", Arguments: strings.Repeat("x", 128)},
		}},
	}}
	assert.GreaterOrEqual(t, assistantContextBytes(messages), 128)
}
