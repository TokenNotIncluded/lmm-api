package middleware

import (
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

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
