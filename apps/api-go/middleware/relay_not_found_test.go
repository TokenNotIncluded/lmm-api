package middleware

import (
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/LIghtJUNction/api.lmm.best/model"
	"github.com/gin-gonic/gin"
	"github.com/glebarez/sqlite"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
	"gorm.io/gorm"
)

func TestTokenAuthHidesMissingAndInvalidCredentialsAsNotFound(t *testing.T) {
	gin.SetMode(gin.TestMode)
	previousDB := model.DB
	previousRedisEnabled := common.RedisEnabled
	common.RedisEnabled = false
	db, err := gorm.Open(sqlite.Open(":memory:"), &gorm.Config{})
	require.NoError(t, err)
	require.NoError(t, db.AutoMigrate(&model.Token{}))
	model.DB = db
	t.Cleanup(func() {
		model.DB = previousDB
		common.RedisEnabled = previousRedisEnabled
	})

	for _, authorization := range []string{"", "Bearer sk-"} {
		t.Run(authorization, func(t *testing.T) {
			router := gin.New()
			router.POST("/v1/messages", TokenAuth(), func(c *gin.Context) {
				c.Status(http.StatusNoContent)
			})
			request := httptest.NewRequest(http.MethodPost, "/v1/messages", nil)
			if authorization != "" {
				request.Header.Set("Authorization", authorization)
			}
			response := httptest.NewRecorder()

			router.ServeHTTP(response, request)
			assert.Equal(t, http.StatusNotFound, response.Code)
			assert.JSONEq(t, `{"message":"Not Found"}`, response.Body.String())
			assert.Empty(t, response.Header().Get("WWW-Authenticate"))
		})
	}
}
