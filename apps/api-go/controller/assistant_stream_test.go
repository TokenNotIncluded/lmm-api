package controller

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/gin-gonic/gin"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestAssistantStreamSessionEmitsIncrementalSafeEvents(t *testing.T) {
	gin.SetMode(gin.TestMode)
	recorder := httptest.NewRecorder()
	context, _ := gin.CreateTestContext(recorder)
	session := newAssistantStreamSession(context.Writer)

	require.NoError(t, session.start())
	require.NoError(t, session.appendContent("实时输出 "))
	require.NoError(t, session.appendContent("内容"))
	require.NoError(t, session.finish([]byte(`{"choices":[{"message":{"content":"实时输出 内容"}}]}`)))

	assert.Equal(t, http.StatusOK, recorder.Code)
	assert.Contains(t, recorder.Header().Get("Content-Type"), "text/event-stream")
	assert.Contains(t, recorder.Body.String(), "event: ready")
	assert.Contains(t, recorder.Body.String(), "event: delta")
	assert.Contains(t, recorder.Body.String(), "event: done")
	assert.Contains(t, recorder.Body.String(), "实时输出 内容")
}

func TestAssistantStreamingRelayWriterParsesSplitSSEChunks(t *testing.T) {
	gin.SetMode(gin.TestMode)
	recorder := httptest.NewRecorder()
	context, _ := gin.CreateTestContext(recorder)
	session := newAssistantStreamSession(context.Writer)
	require.NoError(t, session.start())

	writer := newAssistantStreamingRelayWriter(context.Writer, session)
	first := `data: {"choices":[{"delta":{"content":"hello "}}]}`
	second := "\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"world\"}}]}\n\ndata: [DONE]\n\n"
	_, err := writer.Write([]byte(first[:len(first)-3]))
	require.NoError(t, err)
	_, err = writer.Write([]byte(first[len(first)-3:] + second))
	require.NoError(t, err)

	body, err := writer.responseBody()
	require.NoError(t, err)
	var parsed struct {
		Choices []struct {
			Message struct {
				Content string `json:"content"`
			} `json:"message"`
		} `json:"choices"`
	}
	require.NoError(t, json.Unmarshal(body, &parsed))
	assert.Equal(t, "hello world", parsed.Choices[0].Message.Content)
	assert.NotContains(t, recorder.Body.String(), `"content":"hello world"`)
}

func TestAssistantStreamContentDoesNotExposeEmailAcrossChunks(t *testing.T) {
	gin.SetMode(gin.TestMode)
	recorder := httptest.NewRecorder()
	context, _ := gin.CreateTestContext(recorder)
	session := newAssistantStreamSession(context.Writer)
	require.NoError(t, session.start())
	require.NoError(t, session.appendContent("联系 alice@"))
	require.NoError(t, session.appendContent("example.test 获取帮助"))
	require.NoError(t, session.finish([]byte(`{"choices":[{"message":{"content":"联系 alice@example.test 获取帮助"}}]}`)))

	body := recorder.Body.String()
	assert.NotContains(t, body, "alice@example.test")
	assert.Contains(t, body, "[REDACTED_EMAIL]")
	assert.NotContains(t, strings.ToLower(body), "password")
}
