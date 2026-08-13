// Copyright (c) 2025-2026 QuantumNous. All rights reserved.

package ionet

import (
	"errors"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/QuantumNous/new-api/common"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestDefaultHTTPClientBoundsResponseBody(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, _ *http.Request) {
		_, _ = writer.Write([]byte("12345"))
	}))
	t.Cleanup(server.Close)
	client := &DefaultHTTPClient{client: server.Client(), maxResponseBytes: 4}

	_, err := client.Do(&HTTPRequest{Method: http.MethodGet, URL: server.URL})
	require.Error(t, err)
	assert.True(t, errors.Is(err, common.ErrLimitExceeded))
}

func TestDefaultHTTPClientAcceptsResponseAtBudget(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, _ *http.Request) {
		_, _ = writer.Write([]byte("1234"))
	}))
	t.Cleanup(server.Close)
	client := &DefaultHTTPClient{client: server.Client(), maxResponseBytes: 4}

	response, err := client.Do(&HTTPRequest{Method: http.MethodGet, URL: server.URL})
	require.NoError(t, err)
	assert.Equal(t, []byte("1234"), response.Body)
}
