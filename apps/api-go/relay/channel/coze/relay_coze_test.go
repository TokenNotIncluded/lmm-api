package coze

import (
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/QuantumNous/new-api/common"
	appconstant "github.com/QuantumNous/new-api/constant"
	relaycommon "github.com/QuantumNous/new-api/relay/common"
	"github.com/QuantumNous/new-api/service"
	"github.com/gin-gonic/gin"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func newCozeResponseLimitTestContext(t *testing.T, limit int) *gin.Context {
	t.Helper()
	gin.SetMode(gin.TestMode)
	context, _ := gin.CreateTestContext(httptest.NewRecorder())
	context.Request = httptest.NewRequest(http.MethodPost, "/relay", nil)
	context.Set("coze_conversation_id", "conversation")
	context.Set("coze_chat_id", "chat")
	common.SetContextKey(context, appconstant.ContextKeyResponseByteLimit, limit)
	return context
}

func TestCheckIfChatCompleteAppliesResponseBudget(t *testing.T) {
	service.InitHttpClient()
	server := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, _ *http.Request) {
		writer.WriteHeader(http.StatusOK)
		writer.(http.Flusher).Flush()
		_, _ = io.WriteString(writer, "12345")
	}))
	defer server.Close()

	context := newCozeResponseLimitTestContext(t, 4)
	info := &relaycommon.RelayInfo{ChannelMeta: &relaycommon.ChannelMeta{ChannelBaseUrl: server.URL}}
	err, complete := checkIfChatComplete(&Adaptor{}, context, info)

	assert.False(t, complete)
	assert.ErrorIs(t, err, common.ErrLimitExceeded)
}

func TestCheckIfChatCompletePreservesValidResponse(t *testing.T) {
	service.InitHttpClient()
	var upstreamResponse CozeChatResponse
	upstreamResponse.Data.Status = "completed"
	upstreamResponse.Data.Usage = CozeChatUsage{TokenCount: 3, OutputCount: 2, InputCount: 1}
	responseBody, err := json.Marshal(upstreamResponse)
	require.NoError(t, err)
	server := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, _ *http.Request) {
		_, _ = writer.Write(responseBody)
	}))
	defer server.Close()

	context := newCozeResponseLimitTestContext(t, len(responseBody))
	info := &relaycommon.RelayInfo{ChannelMeta: &relaycommon.ChannelMeta{ChannelBaseUrl: server.URL}}
	err, complete := checkIfChatComplete(&Adaptor{}, context, info)

	require.NoError(t, err)
	assert.True(t, complete)
	assert.Equal(t, 3, context.GetInt("coze_token_count"))
	assert.Equal(t, 2, context.GetInt("coze_output_count"))
	assert.Equal(t, 1, context.GetInt("coze_input_count"))
}

func TestGetChatDetailRejectsKnownOversizeResponse(t *testing.T) {
	service.InitHttpClient()
	server := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, _ *http.Request) {
		writer.Header().Set("Content-Length", "5")
		_, _ = io.WriteString(writer, "12345")
	}))
	defer server.Close()

	context := newCozeResponseLimitTestContext(t, 4)
	info := &relaycommon.RelayInfo{ChannelMeta: &relaycommon.ChannelMeta{ChannelBaseUrl: server.URL}}
	response, err := getChatDetail(&Adaptor{}, context, info)

	assert.Nil(t, response)
	require.Error(t, err)
	assert.True(t, errors.Is(err, common.ErrLimitExceeded))
}
