package middleware

import (
	"crypto/tls"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/QuantumNous/new-api/setting"
	"github.com/gin-gonic/gin"
	"github.com/stretchr/testify/require"
)

func TestAntiRelayAccessIsDisabledByDefault(t *testing.T) {
	withAntiRelayPolicy(t, false, true, false, `[]`, `[]`)

	response := runAntiRelayRequest(t, "198.51.100.10:4000", "example.com", nil, map[string]string{
		"X-Forwarded-For": "203.0.113.10",
	})
	require.Equal(t, http.StatusOK, response.Code)
}

func TestAntiRelayAccessRejectsProxyHeadersFromUntrustedPeer(t *testing.T) {
	withAntiRelayPolicy(t, true, true, false, `[]`, `[]`)

	response := runAntiRelayRequest(t, "198.51.100.10:4000", "example.com", nil, map[string]string{
		"X-Forwarded-For": "203.0.113.10",
	})
	require.Equal(t, http.StatusForbidden, response.Code)
	require.Contains(t, response.Body.String(), "access_policy_rejected")
}

func TestAntiRelayAccessAllowsHeadersFromTrustedReverseProxy(t *testing.T) {
	withAntiRelayPolicy(t, true, true, false, `[]`, `["198.51.100.10/32"]`)

	response := runAntiRelayRequest(t, "198.51.100.10:4000", "example.com", nil, map[string]string{
		"X-Forwarded-For":   "203.0.113.10",
		"X-Forwarded-Proto": "https",
	})
	require.Equal(t, http.StatusOK, response.Code)
}

func TestAntiRelayAccessRejectsBlockedPeerWithoutHeaders(t *testing.T) {
	withAntiRelayPolicy(t, true, false, false, `["198.51.100.0/24"]`, `[]`)

	response := runAntiRelayRequest(t, "198.51.100.10:4000", "example.com", nil, nil)
	require.Equal(t, http.StatusForbidden, response.Code)
}

func TestAntiRelayAccessDoesNotTrustForwardedClientIPAsPeer(t *testing.T) {
	withAntiRelayPolicy(t, true, false, false, `["203.0.113.0/24"]`, `[]`)

	response := runAntiRelayRequest(t, "198.51.100.10:4000", "example.com", nil, map[string]string{
		"X-Forwarded-For": "203.0.113.10",
	})
	require.Equal(t, http.StatusOK, response.Code)
}

func TestAntiRelayAccessCanBeScopedToHTTPSAndPort443(t *testing.T) {
	withAntiRelayPolicy(t, true, true, true, `[]`, `[]`)

	httpResponse := runAntiRelayRequest(t, "198.51.100.10:4000", "example.com", nil, map[string]string{
		"X-Forwarded-For": "203.0.113.10",
	})
	require.Equal(t, http.StatusOK, httpResponse.Code)

	portResponse := runAntiRelayRequest(t, "198.51.100.10:4000", "example.com:443", nil, map[string]string{
		"Via": "1.1 relay.example",
	})
	require.Equal(t, http.StatusForbidden, portResponse.Code)

	httpsResponse := runAntiRelayRequest(t, "198.51.100.10:4000", "example.com", &tls.ConnectionState{}, map[string]string{
		"Via": "1.1 relay.example",
	})
	require.Equal(t, http.StatusForbidden, httpsResponse.Code)
}

func TestAntiRelayAccessRejectsUnknownPeerWhenEnabled(t *testing.T) {
	withAntiRelayPolicy(t, true, false, false, `[]`, `[]`)

	response := runAntiRelayRequest(t, "", "example.com", nil, nil)
	require.Equal(t, http.StatusForbidden, response.Code)
}

func TestAntiRelayAccessTrustedPeerTakesPrecedenceOverBlockedList(t *testing.T) {
	withAntiRelayPolicy(t, true, false, false, `["127.0.0.0/8"]`, `["127.0.0.1/32"]`)

	response := runAntiRelayRequest(t, "127.0.0.1:4000", "example.com", nil, map[string]string{
		"Via": "1.1 local-edge",
	})
	require.Equal(t, http.StatusOK, response.Code)
}

func withAntiRelayPolicy(t *testing.T, enabled, rejectHeaders, httpsOnly bool, blocked, trusted string) {
	t.Helper()
	original := setting.GetAntiRelaySettings()
	t.Cleanup(func() {
		setting.SetAntiRelayEnabled(original.Enabled)
		setting.SetAntiRelayRejectProxyHeaders(original.RejectProxyHeaders)
		setting.SetAntiRelayHTTPSOnly(original.HTTPSOnly)
		require.NoError(t, setting.UpdateAntiRelayBlockedCIDRs(mustCIDRsJSON(original.BlockedCIDRs)))
		require.NoError(t, setting.UpdateAntiRelayTrustedProxyCIDRs(mustCIDRsJSON(original.TrustedProxyCIDRs)))
	})

	setting.SetAntiRelayEnabled(enabled)
	setting.SetAntiRelayRejectProxyHeaders(rejectHeaders)
	setting.SetAntiRelayHTTPSOnly(httpsOnly)
	require.NoError(t, setting.UpdateAntiRelayBlockedCIDRs(blocked))
	require.NoError(t, setting.UpdateAntiRelayTrustedProxyCIDRs(trusted))
}

func mustCIDRsJSON(cidrs []string) string {
	if len(cidrs) == 0 {
		return `[]`
	}
	value := "["
	for index, cidr := range cidrs {
		if index > 0 {
			value += ","
		}
		value += `"` + cidr + `"`
	}
	return value + "]"
}

func runAntiRelayRequest(t *testing.T, remoteAddr, host string, state *tls.ConnectionState, headers map[string]string) *httptest.ResponseRecorder {
	t.Helper()
	gin.SetMode(gin.TestMode)
	router := gin.New()
	router.Use(AntiRelayAccess())
	router.GET("/protected", func(c *gin.Context) {
		c.Status(http.StatusOK)
	})

	request := httptest.NewRequest(http.MethodGet, "http://"+host+"/protected", nil)
	request.RemoteAddr = remoteAddr
	request.Host = host
	request.TLS = state
	for key, value := range headers {
		request.Header.Set(key, value)
	}
	response := httptest.NewRecorder()
	router.ServeHTTP(response, request)
	return response
}
