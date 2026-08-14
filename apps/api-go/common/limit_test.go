package common

import (
	"bytes"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/LIghtJUNction/api.lmm.best/constant"
	"github.com/gorilla/websocket"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestLimitBufferNeverGrowsPastBudget(t *testing.T) {
	buffer := NewLimitBuffer(4)
	_, err := buffer.WriteString("four")
	require.NoError(t, err)
	_, err = buffer.WriteString("!")
	assert.ErrorIs(t, err, ErrLimitExceeded)
	assert.Equal(t, 4, buffer.Len())
	assert.Equal(t, "four", string(buffer.Bytes()))
}

func TestReadAllLimitRejectsOverflow(t *testing.T) {
	data, err := ReadAllLimit(strings.NewReader("1234"), 4)
	require.NoError(t, err)
	assert.Equal(t, "1234", string(data))
	_, err = ReadAllLimit(strings.NewReader("12345"), 4)
	assert.True(t, errors.Is(err, ErrLimitExceeded))
}

func TestLimitBodyStopsAtSourceBudget(t *testing.T) {
	exact, err := io.ReadAll(LimitBody(io.NopCloser(strings.NewReader("1234")), 4))
	require.NoError(t, err)
	assert.Equal(t, "1234", string(exact))

	overflow, err := io.ReadAll(LimitBody(io.NopCloser(strings.NewReader("12345")), 4))
	assert.ErrorIs(t, err, ErrLimitExceeded)
	assert.Equal(t, "1234", string(overflow))
}

func TestLimitHTTPClientCapsResponseBeforeConsumer(t *testing.T) {
	tests := []struct {
		name          string
		payload       string
		contentLength int64
		wantBody      string
		wantReadError bool
		wantDoError   bool
	}{
		{name: "unknown length", payload: "12345", contentLength: -1, wantBody: "1234", wantReadError: true},
		{name: "known overflow", payload: "12345", contentLength: 5, wantDoError: true},
		{name: "exact boundary", payload: "1234", contentLength: 4, wantBody: "1234"},
	}

	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			client := LimitHTTPClient(httpDoerFunc(func(*http.Request) (*http.Response, error) {
				return &http.Response{
					StatusCode:    http.StatusOK,
					ContentLength: test.contentLength,
					Body:          io.NopCloser(strings.NewReader(test.payload)),
				}, nil
			}), 4)

			response, err := client.Do(&http.Request{})
			if test.wantDoError {
				assert.ErrorIs(t, err, ErrLimitExceeded)
				assert.Nil(t, response)
				return
			}
			require.NoError(t, err)
			body, readErr := io.ReadAll(response.Body)
			if test.wantReadError {
				assert.ErrorIs(t, readErr, ErrLimitExceeded)
			} else {
				require.NoError(t, readErr)
			}
			assert.Equal(t, test.wantBody, string(body))
		})
	}
}

type httpDoerFunc func(*http.Request) (*http.Response, error)

func (function httpDoerFunc) Do(request *http.Request) (*http.Response, error) {
	return function(request)
}

func TestMarshalLimitBoundsJSON(t *testing.T) {
	data, err := MarshalLimit(map[string]string{"value": "ok"}, 32)
	require.NoError(t, err)
	assert.JSONEq(t, `{"value":"ok"}`, string(data))
	_, err = MarshalLimit(map[string]string{"value": strings.Repeat("x", 64)}, 32)
	assert.ErrorIs(t, err, ErrLimitExceeded)
}

func TestResponseBodyLimitHasSafeDefault(t *testing.T) {
	assert.Greater(t, ResponseBodyLimit(), int64(0))
}

func TestSetWebSocketReadLimitRejectsOversizedFrame(t *testing.T) {
	previous := constant.MaxResponseBodyMB
	constant.MaxResponseBodyMB = 1
	t.Cleanup(func() { constant.MaxResponseBodyMB = previous })

	serverErr := make(chan error, 1)
	serverUpgrader := websocket.Upgrader{CheckOrigin: func(*http.Request) bool { return true }}
	server := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		conn, err := serverUpgrader.Upgrade(writer, request, nil)
		if err != nil {
			serverErr <- err
			return
		}
		defer conn.Close()
		SetWebSocketReadLimit(conn)
		_, _, err = conn.ReadMessage()
		serverErr <- err
	}))
	defer server.Close()

	client, _, err := websocket.DefaultDialer.Dial("ws"+strings.TrimPrefix(server.URL, "http"), nil)
	require.NoError(t, err)
	defer client.Close()
	// Keep the test small while still exercising the same Gorilla read-limit
	// path used by relay connections in production.
	err = client.WriteMessage(websocket.TextMessage, bytes.Repeat([]byte{'x'}, (1<<20)+1))
	if err != nil {
		// The peer may send the 1009 close before WriteMessage returns; the
		// authoritative assertion is the server-side ErrReadLimit below.
		assert.Error(t, err)
	}

	select {
	case readErr := <-serverErr:
		assert.ErrorIs(t, readErr, websocket.ErrReadLimit)
	case <-time.After(5 * time.Second):
		t.Fatal("timed out waiting for oversized websocket frame rejection")
	}
}

func TestSHA256RequestBodyPreservesBodyAndHonorsLimit(t *testing.T) {
	request, err := http.NewRequest(http.MethodPost, "https://example.com", bytes.NewBufferString("payload"))
	require.NoError(t, err)
	digest, err := SHA256RequestBody(request, 7)
	require.NoError(t, err)
	want := sha256.Sum256([]byte("payload"))
	assert.Equal(t, hex.EncodeToString(want[:]), digest)
	data, err := io.ReadAll(request.Body)
	require.NoError(t, err)
	assert.Equal(t, "payload", string(data))

	request, err = http.NewRequest(http.MethodPost, "https://example.com", bytes.NewBufferString("payload"))
	require.NoError(t, err)
	_, err = SHA256RequestBody(request, 6)
	assert.ErrorIs(t, err, ErrLimitExceeded)
}
