package channel

import (
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"testing"

	common2 "github.com/QuantumNous/new-api/common"
	"github.com/QuantumNous/new-api/constant"
	relaycommon "github.com/QuantumNous/new-api/relay/common"
	"github.com/QuantumNous/new-api/relaykit/types"

	"github.com/gin-gonic/gin"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestUnsupportedOpenAIParameterDetection(t *testing.T) {
	parameter, ok := unsupportedOpenAIParameter([]byte(`{"error":{"message":"temperature is not supported by this model"}}`))
	assert.True(t, ok)
	assert.Equal(t, "temperature", parameter)

	assert.True(t, isVisionCapabilityError([]byte(`{"error":{"message":"deepseek-v4-flash does not support image inputs"}}`)))
	assert.False(t, isVisionCapabilityError([]byte(`{"error":{"message":"invalid image_url"}}`)))
}

func TestRequestBodyWithoutParameterPreservesSemanticFields(t *testing.T) {
	req, err := http.NewRequest(http.MethodPost, "http://example.test/v1/chat/completions", strings.NewReader(`{"model":"gpt-5.6-luna","messages":[{"role":"user","content":"hello"}],"temperature":0.2}`))
	require.NoError(t, err)

	body, changed, err := requestBodyWithoutParameter(req, "temperature")
	require.NoError(t, err)
	require.True(t, changed)
	assert.NotContains(t, string(body), "temperature")
	assert.Contains(t, string(body), "gpt-5.6-luna")
	assert.Contains(t, string(body), "messages")
}

func TestMaybeRetryOpenAICompatibilityErrorOmitsOnlyRejectedParameter(t *testing.T) {
	gin.SetMode(gin.TestMode)
	var mu sync.Mutex
	var received []string
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		body, err := io.ReadAll(r.Body)
		require.NoError(t, err)
		mu.Lock()
		received = append(received, string(body))
		attempt := len(received)
		mu.Unlock()
		if attempt == 1 {
			w.Header().Set("Content-Type", "application/json")
			w.WriteHeader(http.StatusBadRequest)
			_, _ = w.Write([]byte(`{"error":{"message":"temperature is not supported by this model"}}`))
			return
		}
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"ok":true}`))
	}))
	defer server.Close()

	req, err := http.NewRequest(http.MethodPost, server.URL+"/v1/chat/completions", strings.NewReader(`{"model":"gpt-5.6-luna","messages":[{"role":"user","content":"hello"}],"temperature":0.2}`))
	require.NoError(t, err)
	firstResp, err := server.Client().Do(req)
	require.NoError(t, err)

	recorder := httptest.NewRecorder()
	c, _ := gin.CreateTestContext(recorder)
	info := &relaycommon.RelayInfo{RelayFormat: types.RelayFormatOpenAI}
	finalResp, err := maybeRetryOpenAICompatibilityError(c, server.Client(), req, info, firstResp)
	require.NoError(t, err)
	require.NotNil(t, finalResp)
	defer finalResp.Body.Close()
	assert.Equal(t, http.StatusOK, finalResp.StatusCode)

	mu.Lock()
	defer mu.Unlock()
	require.Len(t, received, 2)
	assert.Contains(t, received[0], "temperature")
	assert.NotContains(t, received[1], "temperature")
	assert.Contains(t, received[1], "messages")
	assert.False(t, common2.GetContextKeyBool(c, constant.ContextKeyUpstreamUnsupportedParameter), "successful compatibility retry must not exclude the channel")
}

func TestVisionCapabilityMismatchIsMarkedWithoutStrippingImage(t *testing.T) {
	gin.SetMode(gin.TestMode)
	recorder := httptest.NewRecorder()
	c, _ := gin.CreateTestContext(recorder)
	info := &relaycommon.RelayInfo{RelayFormat: types.RelayFormatOpenAI}
	req, err := http.NewRequest(http.MethodPost, "http://example.test/v1/chat/completions", strings.NewReader(`{"model":"deepseek-v4-flash-0731","messages":[{"role":"user","content":[{"type":"image_url","image_url":{"url":"https://example.test/image.png"}}]}]}`))
	require.NoError(t, err)
	resp := &http.Response{
		StatusCode: http.StatusBadRequest,
		Body:       io.NopCloser(strings.NewReader(`{"error":{"message":"model does not support image inputs"}}`)),
	}

	finalResp, err := maybeRetryOpenAICompatibilityError(c, http.DefaultClient, req, info, resp)
	require.NoError(t, err)
	require.NotNil(t, finalResp)
	defer finalResp.Body.Close()
	body, err := io.ReadAll(finalResp.Body)
	require.NoError(t, err)
	assert.Contains(t, string(body), "image inputs")
	assert.True(t, common2.GetContextKeyBool(c, constant.ContextKeyUpstreamCapabilityMismatch))
	assert.True(t, common2.GetContextKeyBool(c, constant.ContextKeyUpstreamChannelFailure))
}
