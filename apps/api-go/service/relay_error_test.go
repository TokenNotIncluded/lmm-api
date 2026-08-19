package service

import (
	"context"
	"errors"
	"net/http/httptest"
	"testing"

	"github.com/LIghtJUNction/api.lmm.best/relaykit/types"
	"github.com/gin-gonic/gin"
	"github.com/stretchr/testify/assert"
)

func TestShouldRetryRelayErrorSpecificChannelSkipsChannelError(t *testing.T) {
	c, _ := gin.CreateTestContext(httptest.NewRecorder())
	c.Set("specific_channel_id", "1")
	apiErr := types.NewError(errors.New("channel failed"), types.ErrorCodeChannelNoAvailableKey)
	assert.False(t, ShouldRetryRelayError(c, apiErr, 1))
}

func TestShouldRetryRelayErrorEmptySpecificChannelAllowsRetry(t *testing.T) {
	c, _ := gin.CreateTestContext(httptest.NewRecorder())
	c.Set("specific_channel_id", "")
	apiErr := types.NewError(errors.New("channel failed"), types.ErrorCodeChannelNoAvailableKey)
	assert.True(t, ShouldRetryRelayError(c, apiErr, 1))
}

func TestShouldRetryRelayErrorRetriesUpstreamTimeout(t *testing.T) {
	c, _ := gin.CreateTestContext(httptest.NewRecorder())
	apiErr := types.NewErrorWithStatusCode(
		errors.New("upstream timed out"),
		types.ErrorCodeUpstreamTimeout,
		504,
	)
	assert.True(t, ShouldRetryRelayError(c, apiErr, 1))
}

func TestShouldRetryRelayErrorSkipsClientDisconnect(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	c, _ := gin.CreateTestContext(httptest.NewRecorder())
	c.Request = httptest.NewRequest("POST", "/v1/responses", nil).WithContext(ctx)
	cancel()
	apiErr := types.NewErrorWithStatusCode(
		errors.New("client closed request"),
		types.ErrorCodeClientClosedRequest,
		499,
	)
	assert.False(t, ShouldRetryRelayError(c, apiErr, 1))
}
