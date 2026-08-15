package kling

import (
	"errors"
	"io"
	"net/http"
	"net/http/httptest"
	"testing"

	relaycommon "github.com/LIghtJUNction/api.lmm.best/relay/common"
	"github.com/gin-gonic/gin"
	"github.com/stretchr/testify/require"
)

type failingResponseBody struct {
	closed bool
}

func (b *failingResponseBody) Read([]byte) (int, error) {
	return 0, errors.New("upstream read failed")
}

func (b *failingResponseBody) Close() error {
	b.closed = true
	return nil
}

var _ io.ReadCloser = (*failingResponseBody)(nil)

func TestDoResponseClosesUpstreamBodyWhenReadFails(t *testing.T) {
	c, _ := gin.CreateTestContext(httptest.NewRecorder())
	body := &failingResponseBody{}
	resp := &http.Response{Body: body}

	_, _, taskErr := (&TaskAdaptor{}).DoResponse(c, resp, &relaycommon.RelayInfo{})

	require.NotNil(t, taskErr)
	require.True(t, body.closed)
}
