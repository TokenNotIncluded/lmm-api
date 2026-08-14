package router

import (
	"github.com/LIghtJUNction/api.lmm.best/controller"
	"github.com/LIghtJUNction/api.lmm.best/middleware"
	"github.com/gin-gonic/gin"
)

func SetOpenSourceBountyMCPRouter(router *gin.Engine) {
	handler := gin.WrapH(controller.NewOpenSourceBountyMCPHandler())
	mcpRoute := router.Group("/mcp")
	mcpRoute.Use(middleware.RouteTag("mcp"))
	mcpRoute.Use(middleware.GlobalAPIRateLimit())
	mcpRoute.Any("", handler)
	mcpRoute.Any("/", handler)
}
