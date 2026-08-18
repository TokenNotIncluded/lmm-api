/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.
*/

package controller

import (
	"fmt"
	"net"
	"net/http"
	"net/netip"
	"strconv"
	"strings"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/LIghtJUNction/api.lmm.best/setting"
	"github.com/gin-gonic/gin"
)

func ipAccessRoutingError(c *gin.Context, status int, code string, message string) {
	c.Header(accessPolicyResultHeader, accessPolicyDenied)
	c.AbortWithStatusJSON(status, gin.H{
		"success": false,
		"code":    code,
		"message": message,
	})
}

// PersonalAccessIPRetired preserves the historical API path without keeping
// the account-level bypass. Administrators now express exceptions as ordered
// direct rules in IPAccessRoutingRules.
func PersonalAccessIPRetired(c *gin.Context) {
	c.JSON(http.StatusGone, gin.H{
		"success": false,
		"code":    "PERSONAL_IP_ACCESS_RETIRED",
		"message": "personal IP access exceptions are retired; an administrator must configure an IP access routing direct rule",
	})
}

func loopbackPeer(remoteAddr string) bool {
	if host, _, err := net.SplitHostPort(strings.TrimSpace(remoteAddr)); err == nil {
		remoteAddr = host
	}
	addr, err := netip.ParseAddr(strings.TrimSpace(remoteAddr))
	return err == nil && addr.IsLoopback()
}

// CheckIPAccessRoutingPolicy is consumed only by Nginx auth_request. The
// handler requires a loopback peer and evaluates the administrator's ordered
// inbound route rules against edge-owned request metadata.
func CheckIPAccessRoutingPolicy(c *gin.Context) {
	if !loopbackPeer(c.Request.RemoteAddr) {
		ipAccessRoutingError(c, http.StatusForbidden, "INTERNAL_ONLY", "internal policy endpoint")
		return
	}

	originalIP := strings.TrimSpace(c.GetHeader("X-Original-Client-IP"))
	edgeCountry := strings.ToUpper(strings.TrimSpace(c.GetHeader("X-LMM-Edge-Country")))
	if edgeCountry == "" && strings.TrimSpace(c.GetHeader("X-LMM-CN-Source")) == "1" {
		edgeCountry = "CN"
	}
	edgePort, _ := strconv.Atoi(strings.TrimSpace(c.GetHeader("X-LMM-Edge-Port")))
	if edgePort == 0 {
		switch strings.ToLower(strings.TrimSpace(c.GetHeader("X-Forwarded-Proto"))) {
		case "https":
			edgePort = 443
		case "http":
			edgePort = 80
		}
	}

	action, lineNumber, err := setting.EvaluateIPAccessRoute(setting.IPAccessRouteRequest{
		ClientIP:        originalIP,
		CountryCode:     edgeCountry,
		L4Protocol:      "tcp",
		DestinationPort: edgePort,
	})
	if err != nil {
		common.SysError("IP access routing evaluation failed: " + err.Error())
		ipAccessRoutingError(c, http.StatusForbidden, "POLICY_UNAVAILABLE", "access policy unavailable")
		return
	}
	if action == setting.IPAccessRouteReject {
		common.SysLog(fmt.Sprintf("IP access routing rejected client_ip=%s country=%s rule_line=%d", originalIP, edgeCountry, lineNumber))
		ipAccessRoutingError(c, http.StatusForbidden, "IP_ACCESS_ROUTE_REJECTED", "request rejected by IP access routing")
		return
	}
	c.Status(http.StatusNoContent)
}
