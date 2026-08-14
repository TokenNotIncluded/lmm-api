package xunfei

import (
	"encoding/json"
	"errors"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/QuantumNous/new-api/common"
	appconstant "github.com/QuantumNous/new-api/constant"
	"github.com/QuantumNous/new-api/relaykit/dto"
	"github.com/gin-gonic/gin"
	"github.com/gorilla/websocket"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func marshalXunfeiTestResponse(t *testing.T, content string, status int) []byte {
	t.Helper()
	var response XunfeiChatResponse
	response.Payload.Choices.Status = status
	response.Payload.Choices.Text = []XunfeiChatResponseTextItem{{Content: content}}
	data, err := json.Marshal(response)
	require.NoError(t, err)
	return data
}

func runXunfeiResponseSequence(t *testing.T, limit int, messages ...[]byte) ([]XunfeiChatResponse, error) {
	t.Helper()
	upgrader := websocket.Upgrader{CheckOrigin: func(*http.Request) bool { return true }}
	server := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		connection, err := upgrader.Upgrade(writer, request, nil)
		if err != nil {
			return
		}
		defer connection.Close()
		if _, _, err := connection.ReadMessage(); err != nil {
			return
		}
		for _, message := range messages {
			if err := connection.WriteMessage(websocket.TextMessage, message); err != nil {
				return
			}
		}
	}))
	defer server.Close()

	gin.SetMode(gin.TestMode)
	context, _ := gin.CreateTestContext(httptest.NewRecorder())
	common.SetContextKey(context, appconstant.ContextKeyResponseByteLimit, limit)
	authURL := "ws" + strings.TrimPrefix(server.URL, "http")
	dataChan, doneChan, err := xunfeiMakeRequest(context, dto.GeneralOpenAIRequest{}, "general", authURL, "app")
	require.NoError(t, err)

	responses := make([]XunfeiChatResponse, 0, len(messages))
	deadline := time.NewTimer(3 * time.Second)
	defer deadline.Stop()
	for {
		select {
		case response := <-dataChan:
			responses = append(responses, response)
		case responseErr := <-doneChan:
			return responses, responseErr
		case <-deadline.C:
			t.Fatal("timed out waiting for Xunfei response")
			return nil, nil
		}
	}
}

func TestXunfeiResponseBudgetRejectsSingleOversizeMessage(t *testing.T) {
	message := marshalXunfeiTestResponse(t, strings.Repeat("x", 256), 2)
	responses, err := runXunfeiResponseSequence(t, len(message)-1, message)

	assert.Empty(t, responses)
	assert.True(t, errors.Is(err, common.ErrLimitExceeded))
}

func TestXunfeiResponseBudgetRejectsCumulativeOversizeMessages(t *testing.T) {
	first := marshalXunfeiTestResponse(t, "first", 0)
	second := marshalXunfeiTestResponse(t, "second", 2)
	responses, err := runXunfeiResponseSequence(t, len(first)+len(second)-1, first, second)

	require.Len(t, responses, 1)
	assert.Equal(t, "first", responses[0].Payload.Choices.Text[0].Content)
	assert.True(t, errors.Is(err, common.ErrLimitExceeded))
}

func TestXunfeiResponseBudgetPreservesValidMessages(t *testing.T) {
	first := marshalXunfeiTestResponse(t, "first", 0)
	second := marshalXunfeiTestResponse(t, "second", 2)
	responses, err := runXunfeiResponseSequence(t, len(first)+len(second), first, second)

	require.NoError(t, err)
	require.Len(t, responses, 2)
	assert.Equal(t, "second", responses[1].Payload.Choices.Text[0].Content)
}
