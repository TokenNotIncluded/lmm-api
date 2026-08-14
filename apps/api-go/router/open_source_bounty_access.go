package router

import (
	"net/http"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/LIghtJUNction/api.lmm.best/controller"
	"github.com/LIghtJUNction/api.lmm.best/model"
	"github.com/gin-gonic/gin"
)

// requireOpenSourceBountyDeveloperAccess is the bounty-specific authorization
// boundary for every private read and mutation. It deliberately revalidates
// durable user state after UserAuth so access does not depend only on the
// broader console route allowlist or a cached login snapshot.
func requireOpenSourceBountyDeveloperAccess() gin.HandlerFunc {
	return func(c *gin.Context) {
		if err := model.RequireOpenSourceBountyDeveloperAccess(c.GetInt("id")); err != nil {
			if openSourceBountyRedactedRead(c.Request.Method, c.Request.URL.Path) {
				common.ApiSuccess(c, []any{})
				c.Abort()
				return
			}
			if model.OpenSourceBountyErrorCode(err) == "OPEN_SOURCE_BOUNTY_INTERNAL_ERROR" {
				common.SysLog("failed to authorize open-source bounty access: " + err.Error())
			}
			controller.RelayNotFound(c)
			c.Abort()
			return
		}
		c.Next()
	}
}

func openSourceBountyRedactedRead(method string, path string) bool {
	if method != http.MethodGet {
		return false
	}
	switch path {
	case "/api/open-source-bounties/mine",
		"/api/open-source-bounties/accepted",
		"/api/open-source-bounties/notifications",
		"/api/open-source-bounties/tips/received",
		"/api/open-source-bounties/disputes/mine":
		return true
	default:
		return false
	}
}
