package controller

import (
	"context"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/LIghtJUNction/api.lmm.best/model"
	"github.com/modelcontextprotocol/go-sdk/mcp"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

type drawingMCPBearerTransport struct {
	token string
}

func (transport drawingMCPBearerTransport) RoundTrip(request *http.Request) (*http.Response, error) {
	clone := request.Clone(request.Context())
	clone.Header = request.Header.Clone()
	clone.Header.Set("Authorization", "Bearer "+transport.token)
	return http.DefaultTransport.RoundTrip(clone)
}

func TestDrawingMCPAuthenticationDiscoveryAndConfirmation(t *testing.T) {
	_, _, token := setupOpenSourceBountyMCPControllerTest(t)
	require.NoError(t, model.DB.AutoMigrate(&model.Ability{}, &model.Channel{}))
	previousDrawingEnabled := common.DrawingEnabled
	common.DrawingEnabled = true
	t.Cleanup(func() { common.DrawingEnabled = previousDrawingEnabled })

	server := httptest.NewServer(NewDrawingMCPHandler())
	t.Cleanup(server.Close)

	for _, test := range []struct {
		name  string
		token string
	}{
		{name: "missing bearer"},
		{name: "invalid bearer", token: "lmm_mcp_invalid"},
	} {
		t.Run(test.name, func(t *testing.T) {
			request, err := http.NewRequest(http.MethodPost, server.URL, strings.NewReader(`{}`))
			require.NoError(t, err)
			request.Header.Set("Content-Type", "application/json")
			if test.token != "" {
				request.Header.Set("Authorization", "Bearer "+test.token)
			}
			response, err := http.DefaultClient.Do(request)
			require.NoError(t, err)
			defer response.Body.Close()
			assert.Equal(t, http.StatusUnauthorized, response.StatusCode)
		})
	}

	client := mcp.NewClient(&mcp.Implementation{Name: "drawing-mcp-test", Version: "1.0.0"}, &mcp.ClientOptions{
		MultiRoundTrip: &mcp.MultiRoundTripOptions{Disabled: true},
	})
	session, err := client.Connect(context.Background(), &mcp.StreamableClientTransport{
		Endpoint:             server.URL,
		HTTPClient:           &http.Client{Transport: drawingMCPBearerTransport{token: token}},
		DisableStandaloneSSE: true,
	}, nil)
	require.NoError(t, err)
	t.Cleanup(func() { _ = session.Close() })

	tools, err := session.ListTools(context.Background(), nil)
	require.NoError(t, err)
	assert.Len(t, tools.Tools, 2)
	names := []string{tools.Tools[0].Name, tools.Tools[1].Name}
	assert.ElementsMatch(t, []string{"drawing.list_capabilities", "drawing.generate"}, names)

	capabilities, err := session.CallTool(context.Background(), &mcp.CallToolParams{
		Name: "drawing.list_capabilities", Arguments: map[string]any{},
	})
	require.NoError(t, err)
	assert.False(t, capabilities.IsError)

	invalid, err := session.CallTool(context.Background(), &mcp.CallToolParams{
		Name: "drawing.generate", Arguments: map[string]any{"prompt": ""},
	})
	require.NoError(t, err)
	assert.True(t, invalid.IsError)
}
