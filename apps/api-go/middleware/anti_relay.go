package middleware

import (
	"fmt"
	"net"
	"net/http"
	"net/netip"
	"strconv"
	"strings"

	"github.com/QuantumNous/new-api/common"
	"github.com/QuantumNous/new-api/logger"
	"github.com/QuantumNous/new-api/setting"

	"github.com/gin-gonic/gin"
)

var antiRelayProxyHeaders = []string{
	"Forwarded",
	"Via",
	"Proxy-Connection",
	"X-Forwarded-For",
	"X-Forwarded-Host",
	"X-Forwarded-Port",
	"X-Forwarded-Proto",
	"X-Forwarded-Protocol",
	"X-Forwarded-Scheme",
	"X-Forwarded-Server",
	"X-Real-IP",
	"X-Client-IP",
	"Client-IP",
	"CF-Connecting-IP",
	"True-Client-IP",
	"Fastly-Client-IP",
	"X-Original-Forwarded-For",
	"X-Original-Client-IP",
	"X-Original-URL",
	"X-Original-URI",
	"X-Proxy-Connection",
	"X-Proxy-Host",
	"X-Proxy-User",
	"X-Envoy-External-Address",
	"X-Cluster-Client-IP",
}

// AntiRelayAccess applies the operator-configured ingress policy before any
// route-specific authentication or request processing. It deliberately uses
// Request.RemoteAddr as the peer identity; forwarding headers are signals, not
// a source of truth, unless the peer itself is in the trusted proxy list.
func AntiRelayAccess() gin.HandlerFunc {
	return func(c *gin.Context) {
		policy := setting.GetAntiRelaySettings()
		if !policy.Enabled || (policy.HTTPSOnly && !isHTTPSRequest(c.Request)) {
			c.Next()
			return
		}

		peer, ok := parseRemotePeer(c.Request.RemoteAddr)
		if !ok {
			rejectAntiRelayRequest(c, "unknown_peer", "")
			return
		}

		// A trusted reverse proxy is the only component allowed to add the
		// forwarding headers that normal deployments use. Trusted peers take
		// precedence over the blocked list so an operator cannot lock out the
		// local edge by accidentally overlapping the two lists.
		if policy.IsTrustedProxy(peer) {
			c.Next()
			return
		}

		if policy.IsBlockedPeer(peer) {
			rejectAntiRelayRequest(c, "blocked_peer", peer.String())
			return
		}

		if policy.RejectProxyHeaders {
			if header := firstAntiRelayProxyHeader(c.Request); header != "" {
				rejectAntiRelayRequest(c, "proxy_header:"+header, peer.String())
				return
			}
		}

		c.Next()
	}
}

func parseRemotePeer(remoteAddr string) (netip.Addr, bool) {
	remoteAddr = strings.TrimSpace(remoteAddr)
	if host, _, err := net.SplitHostPort(remoteAddr); err == nil {
		remoteAddr = host
	}
	remoteAddr = strings.Trim(remoteAddr, "[]")
	address, err := netip.ParseAddr(remoteAddr)
	if err != nil {
		return netip.Addr{}, false
	}
	return address.Unmap(), true
}

func firstAntiRelayProxyHeader(request *http.Request) string {
	for _, header := range antiRelayProxyHeaders {
		for _, value := range request.Header.Values(header) {
			if strings.TrimSpace(value) != "" {
				return header
			}
		}
	}
	return ""
}

func isHTTPSRequest(request *http.Request) bool {
	if request == nil {
		return false
	}
	if request.TLS != nil {
		return true
	}
	if request.URL != nil && strings.EqualFold(request.URL.Scheme, "https") {
		return true
	}
	if antiRelayRequestPort(request) == 443 {
		return true
	}
	if strings.EqualFold(strings.TrimSpace(request.Header.Get("X-Forwarded-Proto")), "https") {
		return true
	}
	if strings.TrimSpace(request.Header.Get("X-Forwarded-Port")) == "443" {
		return true
	}
	return strings.Contains(strings.ToLower(request.Header.Get("Forwarded")), "proto=https")
}

func rejectAntiRelayRequest(c *gin.Context, reason string, peer string) {
	path := ""
	if c.Request.URL != nil {
		path = c.Request.URL.Path
	}
	logger.LogWarn(c.Request.Context(), fmt.Sprintf(
		"anti-relay rejected reason=%s peer=%s method=%s path=%s",
		reason,
		peer,
		c.Request.Method,
		path,
	))

	message := "request rejected by access policy"
	if requestID := c.GetString(common.RequestIdKey); requestID != "" {
		message = common.MessageWithRequestId(message, requestID)
	}
	c.Header("Cache-Control", "no-store")
	c.AbortWithStatusJSON(http.StatusForbidden, gin.H{
		"error": gin.H{
			"message": message,
			"type":    "new_api_error",
			"code":    "access_policy_rejected",
		},
	})
}

func antiRelayRequestPort(request *http.Request) int {
	if request == nil {
		return 0
	}
	if request.TLS != nil {
		return 443
	}
	if request.URL != nil {
		if port := request.URL.Port(); port != "" {
			parsed, err := strconv.Atoi(port)
			if err == nil {
				return parsed
			}
		}
	}
	if _, port, err := net.SplitHostPort(request.Host); err == nil {
		parsed, err := strconv.Atoi(port)
		if err == nil {
			return parsed
		}
	}
	return 0
}
