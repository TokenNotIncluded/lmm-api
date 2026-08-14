package middleware

import (
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/LIghtJUNction/api.lmm.best/constant"
	"github.com/LIghtJUNction/api.lmm.best/model"
	"github.com/gin-gonic/gin"
	"github.com/stretchr/testify/assert"
)

func TestWebSocketSubprotocolAuthorization(t *testing.T) {
	tests := []struct {
		name      string
		protocols string
		wantKey   string
		wantOK    bool
	}{
		{name: "responses only", protocols: "responses"},
		{name: "realtime only", protocols: "realtime"},
		{name: "responses key", protocols: "responses, openai-insecure-api-key.sk-test", wantKey: "sk-test", wantOK: true},
		{name: "empty key", protocols: "responses, openai-insecure-api-key."},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			key, ok := apiKeyFromWebSocketSubprotocol(tt.protocols)
			assert.Equal(t, tt.wantOK, ok)
			assert.Equal(t, tt.wantKey, key)
		})
	}
}

func TestResponsesSubprotocolDoesNotReplaceAuthorization(t *testing.T) {
	header := http.Header{}
	header.Set("Authorization", "Bearer sk-original")
	header.Set("Sec-WebSocket-Protocol", "responses")
	assert.False(t, applyWebSocketSubprotocolAuthorization(header))
	assert.Equal(t, "Bearer sk-original", header.Get("Authorization"))

	header.Set("Sec-WebSocket-Protocol", "responses, openai-insecure-api-key.sk-protocol")
	assert.True(t, applyWebSocketSubprotocolAuthorization(header))
	assert.Equal(t, "Bearer sk-protocol", header.Get("Authorization"))
}

func TestSetupContextForTokenDoesNotSetEmptySpecificChannel(t *testing.T) {
	context, _ := gin.CreateTestContext(httptest.NewRecorder())
	token := &model.Token{Id: 7, UserId: 9, Key: "plain-token", Name: "plain"}

	assert.NoError(t, SetupContextForToken(context, token))
	assert.Empty(t, common.GetContextKeyString(context, constant.ContextKeyTokenSpecificChannelId),
		"ordinary tokens must use normal channel selection")
}
