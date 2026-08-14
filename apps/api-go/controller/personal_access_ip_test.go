package controller

import (
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/gin-gonic/gin"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestLoopbackPeerAcceptsOnlyLoopbackAddresses(t *testing.T) {
	assert.True(t, loopbackPeer("127.0.0.1:3000"))
	assert.True(t, loopbackPeer("[::1]:3000"))
	assert.False(t, loopbackPeer("10.0.0.2:3000"))
	assert.False(t, loopbackPeer("198.51.100.2:3000"))
	assert.False(t, loopbackPeer("not-an-address"))
}

func TestPersonalAccessIPPolicyMarksOnlyDeniedRequests(t *testing.T) {
	gin.SetMode(gin.TestMode)

	t.Run("anonymous CN request is marked denied", func(t *testing.T) {
		recorder := httptest.NewRecorder()
		context, _ := gin.CreateTestContext(recorder)
		context.Request = httptest.NewRequest(http.MethodGet, "/internal/access-ip-policy", nil)
		context.Request.RemoteAddr = "127.0.0.1:42000"
		context.Request.Header.Set("X-LMM-CN-Source", "1")

		CheckPersonalAccessIPPolicy(context)

		assert.Equal(t, http.StatusUnauthorized, context.Writer.Status())
		assert.Equal(t, accessPolicyDenied, recorder.Header().Get(accessPolicyResultHeader))
	})

	t.Run("non-CN request passes without denial marker", func(t *testing.T) {
		recorder := httptest.NewRecorder()
		context, _ := gin.CreateTestContext(recorder)
		context.Request = httptest.NewRequest(http.MethodGet, "/internal/access-ip-policy", nil)
		context.Request.RemoteAddr = "127.0.0.1:42000"

		CheckPersonalAccessIPPolicy(context)

		assert.Equal(t, http.StatusNoContent, context.Writer.Status())
		assert.Empty(t, recorder.Header().Get(accessPolicyResultHeader))
	})

	t.Run("public peer cannot spoof edge headers", func(t *testing.T) {
		recorder := httptest.NewRecorder()
		context, _ := gin.CreateTestContext(recorder)
		context.Request = httptest.NewRequest(http.MethodGet, "/internal/access-ip-policy", nil)
		context.Request.RemoteAddr = "198.51.100.2:42000"
		context.Request.Header.Set("X-LMM-Edge-Country", "US")
		context.Request.Header.Set("X-LMM-CN-Source", "0")

		CheckPersonalAccessIPPolicy(context)

		assert.Equal(t, http.StatusForbidden, context.Writer.Status())
		assert.Equal(t, accessPolicyDenied, recorder.Header().Get(accessPolicyResultHeader))
	})

	t.Run("disabled policy allows blocked country", func(t *testing.T) {
		previous := common.IsRegionAccessPolicyEnabled()
		common.SetRegionAccessPolicyEnabled(false)
		t.Cleanup(func() { common.SetRegionAccessPolicyEnabled(previous) })

		recorder := httptest.NewRecorder()
		context, _ := gin.CreateTestContext(recorder)
		context.Request = httptest.NewRequest(http.MethodGet, "/internal/access-ip-policy", nil)
		context.Request.RemoteAddr = "127.0.0.1:42000"
		context.Request.Header.Set("X-LMM-CN-Source", "1")

		CheckPersonalAccessIPPolicy(context)

		assert.Equal(t, http.StatusNoContent, context.Writer.Status())
		assert.Empty(t, recorder.Header().Get(accessPolicyResultHeader))
	})
}

func TestAccessPolicyErrorPageRequiresCapturedDenial(t *testing.T) {
	gin.SetMode(gin.TestMode)

	request := func(remoteAddr string, headers map[string]string) (int, *httptest.ResponseRecorder) {
		recorder := httptest.NewRecorder()
		context, _ := gin.CreateTestContext(recorder)
		context.Request = httptest.NewRequest(http.MethodGet, "/internal/errors/access-policy", nil)
		context.Request.RemoteAddr = remoteAddr
		for name, value := range headers {
			context.Request.Header.Set(name, value)
		}
		GetAccessPolicyErrorPage(context)
		return context.Writer.Status(), recorder
	}

	validHeaders := map[string]string{
		"X-LMM-Internal-Error":   accessPolicyErrorHeader,
		accessPolicyResultHeader: accessPolicyDenied,
		"X-Original-Client-IP":   "203.0.113.42",
		"X-LMM-CN-Source":        "1",
		"X-LMM-Edge-Country":     "CN",
		"User-Agent":             "Mozilla/5.0 TestBrowser",
	}
	status, response := request("127.0.0.1:42000", validHeaders)
	require.Equal(t, http.StatusUnavailableForLegalReasons, status)
	assert.Contains(t, response.Header().Get("Content-Type"), "text/html")
	body := response.Body.String()
	assert.Contains(t, body, "您所在地区暂不支持直接访问")
	assert.Contains(t, body, "疑难解答")
	assert.Contains(t, body, "203.0.113.42")
	assert.Contains(t, body, "IPv4")
	assert.Contains(t, body, "region_denied")
	assert.NotContains(t, body, "符合条件的账号")

	status, _ = request("127.0.0.1:42000", map[string]string{
		"X-LMM-Internal-Error": accessPolicyErrorHeader,
	})
	assert.Equal(t, http.StatusNotFound, status)

	status, _ = request("198.51.100.2:42000", validHeaders)
	assert.Equal(t, http.StatusNotFound, status)
}
