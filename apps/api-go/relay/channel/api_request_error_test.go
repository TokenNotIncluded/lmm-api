package channel

import (
	"context"
	"errors"
	"net/http"
	"net/http/httptest"
	"testing"

	common2 "github.com/LIghtJUNction/api.lmm.best/common"
	appconstant "github.com/LIghtJUNction/api.lmm.best/constant"
	"github.com/LIghtJUNction/api.lmm.best/relaykit/types"
	"github.com/gin-gonic/gin"
	"github.com/stretchr/testify/require"
)

func TestClassifyUpstreamTransportError(t *testing.T) {
	tests := []struct {
		name              string
		err               error
		cancelClient      bool
		wantStatus        int
		wantCode          types.ErrorCode
		wantSkipRetry     bool
		wantChannelFailed bool
	}{
		{
			name:              "client disconnect",
			err:               context.Canceled,
			cancelClient:      true,
			wantStatus:        499,
			wantCode:          types.ErrorCodeClientClosedRequest,
			wantSkipRetry:     true,
			wantChannelFailed: false,
		},
		{
			name:              "upstream cancellation",
			err:               context.Canceled,
			wantStatus:        http.StatusServiceUnavailable,
			wantCode:          types.ErrorCodeUpstreamCanceled,
			wantChannelFailed: true,
		},
		{
			name:              "upstream timeout",
			err:               context.DeadlineExceeded,
			wantStatus:        http.StatusGatewayTimeout,
			wantCode:          types.ErrorCodeUpstreamTimeout,
			wantChannelFailed: true,
		},
		{
			name:              "transport failure",
			err:               errors.New("connection reset by peer"),
			wantStatus:        http.StatusBadGateway,
			wantCode:          types.ErrorCodeUpstreamTransport,
			wantChannelFailed: true,
		},
	}

	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			gin.SetMode(gin.TestMode)
			ctx, cancel := context.WithCancel(context.Background())
			defer cancel()
			c, _ := gin.CreateTestContext(httptest.NewRecorder())
			c.Request = httptest.NewRequest(http.MethodPost, "/v1/responses", nil).WithContext(ctx)
			if test.cancelClient {
				cancel()
			}

			apiErr := classifyUpstreamTransportError(c, test.err)
			require.Equal(t, test.wantStatus, apiErr.StatusCode)
			require.Equal(t, test.wantCode, apiErr.GetErrorCode())
			require.Equal(t, test.wantSkipRetry, types.IsSkipRetryError(apiErr))
			require.Equal(t, test.wantChannelFailed, common2.GetContextKeyBool(c, appconstant.ContextKeyUpstreamChannelFailure))
		})
	}
}
