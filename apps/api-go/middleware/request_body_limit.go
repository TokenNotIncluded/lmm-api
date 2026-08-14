package middleware

import (
	"errors"
	"io"
	"net/http"

	"github.com/QuantumNous/new-api/common"
	"github.com/gin-gonic/gin"
)

// RequestBodyLimit applies a route-specific post-decompression ceiling without
// eagerly copying the body. Handlers can detect common.ErrRequestBodyTooLarge
// through the existing reusable body decoder.
func RequestBodyLimit(maxBytes int64) gin.HandlerFunc {
	return func(c *gin.Context) {
		if maxBytes <= 0 || c.Request.Body == nil {
			c.Next()
			return
		}
		if c.Request.ContentLength > maxBytes {
			c.AbortWithStatus(http.StatusRequestEntityTooLarge)
			return
		}
		c.Request.Body = http.MaxBytesReader(c.Writer, c.Request.Body, maxBytes)
		c.Next()
	}
}

func AnonymousRequestBodyLimit() gin.HandlerFunc {
	return RequestBodyLimit(common.GetAnonymousRequestBodyLimitBytes())
}

func readAnonymousRequestBody(body io.Reader, maxBytes int64) ([]byte, error) {
	data, err := common.ReadAllLimit(body, maxBytes)
	if errors.Is(err, common.ErrLimitExceeded) {
		return nil, common.ErrRequestBodyTooLarge
	}
	return data, err
}
