package router

import (
	"testing"

	"github.com/gin-gonic/gin"
	"github.com/stretchr/testify/assert"
)

func TestRelayRouterRegistersPublicAssistantPresetEndpoints(t *testing.T) {
	gin.SetMode(gin.TestMode)
	engine := gin.New()
	SetRelayRouter(engine)

	routes := make(map[string]bool)
	for _, route := range engine.Routes() {
		routes[route.Method+" "+route.Path] = true
	}
	assert.True(t, routes["GET /api/assistant/pre-conversation-presets"])
	assert.True(t, routes["POST /api/assistant/pre-conversation-presets/:id/click"])
}
