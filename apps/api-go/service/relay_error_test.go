package service

import (
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
