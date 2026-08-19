package router

import (
	"github.com/LIghtJUNction/api.lmm.best/controller"
	"github.com/LIghtJUNction/api.lmm.best/middleware"
	"github.com/gin-gonic/gin"
)

// SetDrawingMCPRouter mounts the drawing-only personal MCP endpoint. It is
// intentionally separate from /mcp so agents can discover only image tools
// when they are configured for the drawing workbench.
func SetDrawingMCPRouter(router *gin.Engine) {
	handler := gin.WrapH(controller.NewDrawingMCPHandler())
	mcpRoute := router.Group("/mcp/drawing")
	mcpRoute.Use(middleware.RouteTag("mcp"))
	mcpRoute.Use(middleware.GlobalAPIRateLimit())
	mcpRoute.Any("", handler)
	mcpRoute.Any("/", handler)
}
