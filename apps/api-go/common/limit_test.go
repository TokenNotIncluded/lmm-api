package common

import (
	"bytes"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"io"
	"net/http"
	"strings"
	"testing"

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
