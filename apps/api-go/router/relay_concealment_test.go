package router

import (
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/gin-gonic/gin"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestExternalFrontendDoesNotRevealRelayRoutes(t *testing.T) {
	gin.SetMode(gin.TestMode)
	previousMasterNode := common.IsMasterNode
	common.IsMasterNode = false
	t.Cleanup(func() { common.IsMasterNode = previousMasterNode })
	t.Setenv("FRONTEND_BASE_URL", "https://frontend.example")

	engine := gin.New()
	require.NoError(t, SetRouter(engine))

	for _, method := range []string{http.MethodGet, http.MethodHead} {
		t.Run(method, func(t *testing.T) {
			known := httptest.NewRecorder()
			engine.ServeHTTP(
				known,
				httptest.NewRequest(method, "/v1/messages", nil),
			)
			unknown := httptest.NewRecorder()
			engine.ServeHTTP(
				unknown,
				httptest.NewRequest(method, "/v1/not-a-relay-route", nil),
			)

			assert.Equal(t, http.StatusNotFound, known.Code)
			assert.Equal(t, known.Code, unknown.Code)
			assert.Equal(t, known.Body.String(), unknown.Body.String())
			assert.Empty(t, known.Header().Get("Location"))
			assert.Empty(t, unknown.Header().Get("Location"))
		})
	}

	t.Run(http.MethodOptions, func(t *testing.T) {
		responses := make([]*httptest.ResponseRecorder, 0, 2)
		for _, path := range []string{"/v1/messages", "/v1/not-a-relay-route"} {
			request := httptest.NewRequest(http.MethodOptions, path, nil)
			request.Header.Set("Origin", "https://client.example")
			request.Header.Set("Access-Control-Request-Method", http.MethodPost)
			response := httptest.NewRecorder()
			engine.ServeHTTP(response, request)
			responses = append(responses, response)
		}

		require.Len(t, responses, 2)
		assert.Equal(t, http.StatusNoContent, responses[0].Code)
		assert.Equal(t, responses[0].Code, responses[1].Code)
		assert.Equal(
			t,
			responses[0].Header().Get("Access-Control-Allow-Methods"),
			responses[1].Header().Get("Access-Control-Allow-Methods"),
		)
		assert.Equal(t, responses[0].Body.String(), responses[1].Body.String())
	})
}

func TestExternalFrontendKeepsEveryBackendFamilyOffThePublicSurface(t *testing.T) {
	gin.SetMode(gin.TestMode)
	previousMasterNode := common.IsMasterNode
	common.IsMasterNode = false
	t.Cleanup(func() { common.IsMasterNode = previousMasterNode })
	t.Setenv("FRONTEND_BASE_URL", "https://frontend.example")

	engine := gin.New()
	require.NoError(t, SetRouter(engine))

	backendPaths := []string{
		"/api/not-a-route",
		"/assets/not-a-route",
		"/mcp/not-a-route",
		"/v1/not-a-route",
		"/v1beta/not-a-route",
		"/pg/not-a-route",
		"/mj/not-a-route",
		"/suno/not-a-route",
		"/kling/v1/not-a-route",
		"/jimeng/not-a-route",
		"/dashboard/billing/subscription",
		"/dashboard/billing/usage",
		"/fast/mj/not-a-route",
	}
	for _, path := range backendPaths {
		t.Run(path, func(t *testing.T) {
			response := httptest.NewRecorder()
			request := httptest.NewRequest(http.MethodGet, path+"?probe=1", nil)

			engine.ServeHTTP(response, request)

			assert.Equal(t, http.StatusNotFound, response.Code)
			assert.JSONEq(t, `{"message":"Not Found"}`, response.Body.String())
			assert.Empty(t, response.Header().Get("Location"))
		})
	}

	frontendResponse := httptest.NewRecorder()
	engine.ServeHTTP(
		frontendResponse,
		httptest.NewRequest(http.MethodGet, "/challenges?status=open", nil),
	)
	assert.Equal(t, http.StatusMovedPermanently, frontendResponse.Code)
	assert.Equal(
		t,
		"https://frontend.example/challenges?status=open",
		frontendResponse.Header().Get("Location"),
	)
}
