package middleware

import (
	"fmt"
	"net/http"
	"runtime/debug"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/gin-gonic/gin"
)

func RelayPanicRecover() gin.HandlerFunc {
	return func(c *gin.Context) {
		defer func() {
			if err := recover(); err != nil {
				common.SysLog(fmt.Sprintf("panic detected: %v", err))
				common.SysLog(fmt.Sprintf("stacktrace from panic: %s", string(debug.Stack())))
				c.JSON(http.StatusInternalServerError, gin.H{
					"error": gin.H{
						"message": "The server encountered an internal error. Please try again or contact support.",
						"type":    "new_api_panic",
					},
				})
				c.Abort()
			}
		}()
		c.Next()
	}
}
