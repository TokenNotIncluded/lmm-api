/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.
*/

package middleware

import (
	"fmt"
	"net/http"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/LIghtJUNction/api.lmm.best/logger"
	"github.com/LIghtJUNction/api.lmm.best/setting"
	"github.com/gin-gonic/gin"
)

var globalIPWhitelistHealthPaths = map[string]struct{}{
	"/api/status":        {},
	"/api/uptime/status": {},
	"/api/livez":         {},
	"/api/readyz":        {},
}

func GlobalIPWhitelist() gin.HandlerFunc {
	return func(c *gin.Context) {
		if _, isHealthProbe := globalIPWhitelistHealthPaths[c.Request.URL.Path]; isHealthProbe {
			c.Next()
			return
		}

		policy := setting.GetGlobalIPWhitelistSettings()
		clientIP := c.ClientIP()
		if policy.Allows(clientIP) {
			c.Next()
			return
		}

		logger.LogWarn(c.Request.Context(), fmt.Sprintf(
			"global IP whitelist rejected client_ip=%s method=%s path=%s",
			clientIP,
			c.Request.Method,
			c.Request.URL.Path,
		))
		message := "request rejected by global IP whitelist"
		if requestID := c.GetString(common.RequestIdKey); requestID != "" {
			message = common.MessageWithRequestId(message, requestID)
		}
		c.AbortWithStatusJSON(http.StatusForbidden, gin.H{
			"success": false,
			"message": message,
		})
	}
}
