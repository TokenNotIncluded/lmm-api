package middleware

import (
	"bytes"
	"compress/gzip"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/LIghtJUNction/api.lmm.best/constant"
	"github.com/gin-gonic/gin"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestRequestBodyLimitRejectsContentLengthBeforeHandler(t *testing.T) {
	gin.SetMode(gin.TestMode)
	router := gin.New()
	called := false
	router.POST("/", RequestBodyLimit(4), func(c *gin.Context) { called = true })
	request := httptest.NewRequest(http.MethodPost, "/", strings.NewReader("12345"))
	response := httptest.NewRecorder()
	router.ServeHTTP(response, request)
	assert.Equal(t, http.StatusRequestEntityTooLarge, response.Code)
	assert.False(t, called)
}

func TestRequestBodyLimitStreamsWithinBudget(t *testing.T) {
	gin.SetMode(gin.TestMode)
	router := gin.New()
	router.POST("/", RequestBodyLimit(4), func(c *gin.Context) {
		data, err := readAnonymousRequestBody(c.Request.Body, 4)
		require.NoError(t, err)
		c.String(http.StatusOK, string(data))
	})
	request := httptest.NewRequest(http.MethodPost, "/", strings.NewReader("1234"))
	response := httptest.NewRecorder()
	router.ServeHTTP(response, request)
	assert.Equal(t, http.StatusOK, response.Code)
	assert.Equal(t, "1234", response.Body.String())
}

func TestRequestBodyLimitRejectsOversizedDecompressedPayload(t *testing.T) {
	gin.SetMode(gin.TestMode)
	router := gin.New()
	mutated := false
	router.POST("/", DecompressRequestMiddleware(), RequestBodyLimit(4), func(c *gin.Context) {
		var payload map[string]string
		if err := common.DecodeJson(c.Request.Body, &payload); err != nil {
			if common.IsRequestBodyTooLargeError(err) {
				c.Status(http.StatusRequestEntityTooLarge)
				return
			}
			c.Status(http.StatusBadRequest)
			return
		}
		mutated = true
		c.Status(http.StatusOK)
	})

	var compressed bytes.Buffer
	writer := gzip.NewWriter(&compressed)
	_, err := writer.Write([]byte(`{"name":"too-large-after-inflation"}`))
	require.NoError(t, err)
	require.NoError(t, writer.Close())

	request := httptest.NewRequest(http.MethodPost, "/", &compressed)
	request.Header.Set("Content-Encoding", "gzip")
	// An unknown transfer length forces the limit to exercise the wrapped,
	// decompressed reader instead of the conservative Content-Length fast path.
	request.ContentLength = -1
	response := httptest.NewRecorder()
	router.ServeHTTP(response, request)

	assert.Equal(t, http.StatusRequestEntityTooLarge, response.Code)
	assert.False(t, mutated)
}

func TestDecompressRequestMiddlewareRejectsOversizedCompressedStream(t *testing.T) {
	gin.SetMode(gin.TestMode)
	previousMaxRequestBodyMB := constant.MaxRequestBodyMB
	constant.MaxRequestBodyMB = 1
	t.Cleanup(func() { constant.MaxRequestBodyMB = previousMaxRequestBodyMB })

	router := gin.New()
	router.POST("/", DecompressRequestMiddleware(), func(c *gin.Context) {
		_, err := io.ReadAll(c.Request.Body)
		if common.IsRequestBodyTooLargeError(err) {
			c.Status(http.StatusRequestEntityTooLarge)
			return
		}
		require.NoError(t, err)
		c.Status(http.StatusOK)
	})

	var member bytes.Buffer
	writer := gzip.NewWriter(&member)
	require.NoError(t, writer.Close())
	require.NotEmpty(t, member.Bytes())

	// Concatenated empty gzip members decode to no payload. Their raw stream
	// still exceeds the configured 1 MiB limit, so this specifically proves
	// the compressed-source ceiling rather than the post-decompression one.
	members := (1<<20)/member.Len() + 1
	compressed := bytes.Repeat(member.Bytes(), members)
	require.Greater(t, len(compressed), 1<<20)

	request := httptest.NewRequest(http.MethodPost, "/", bytes.NewReader(compressed))
	request.Header.Set("Content-Encoding", "gzip")
	request.ContentLength = -1
	response := httptest.NewRecorder()
	router.ServeHTTP(response, request)

	assert.Equal(t, http.StatusRequestEntityTooLarge, response.Code)
}
