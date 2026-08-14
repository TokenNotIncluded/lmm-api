package middleware

import (
	"context"
	"io"
	"net/http"
	"net/url"
	"strings"
	"testing"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

type turnstileRoundTripper func(*http.Request) (*http.Response, error)

func (f turnstileRoundTripper) RoundTrip(request *http.Request) (*http.Response, error) {
	return f(request)
}

func TestVerifyTurnstileTokenDoesNotSendProxyAddress(t *testing.T) {
	previousClient := turnstileHTTPClient
	previousURL := turnstileVerifyURL
	previousSecret := common.TurnstileSecretKey
	t.Cleanup(func() {
		turnstileHTTPClient = previousClient
		turnstileVerifyURL = previousURL
		common.TurnstileSecretKey = previousSecret
	})

	turnstileVerifyURL = "https://turnstile.test/siteverify"
	common.TurnstileSecretKey = "test-secret"
	turnstileHTTPClient = &http.Client{Transport: turnstileRoundTripper(func(request *http.Request) (*http.Response, error) {
		body, err := io.ReadAll(request.Body)
		require.NoError(t, err)
		form, err := url.ParseQuery(string(body))
		require.NoError(t, err)
		assert.Equal(t, []string{"test-secret"}, form["secret"])
		assert.Equal(t, []string{"test-token"}, form["response"])
		assert.NotContains(t, form, "remoteip")
		return &http.Response{
			StatusCode: http.StatusOK,
			Body:       io.NopCloser(strings.NewReader(`{"success":true,"hostname":"lmm.best"}`)),
			Header:     make(http.Header),
		}, nil
	})}

	result, err := verifyTurnstileToken(context.Background(), "test-token")
	require.NoError(t, err)
	assert.True(t, result.Success)
	assert.Equal(t, "lmm.best", result.Hostname)
}
