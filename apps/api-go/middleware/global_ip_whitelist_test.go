/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.
*/

package middleware

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/LIghtJUNction/api.lmm.best/setting"
	"github.com/gin-gonic/gin"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestGlobalIPWhitelistMiddleware(t *testing.T) {
	original := setting.GetGlobalIPWhitelistSettings()
	t.Cleanup(func() {
		encoded, err := json.Marshal(original.CIDRs)
		require.NoError(t, err)
		require.NoError(t, setting.UpdateGlobalIPWhitelistCIDRs(string(encoded)))
		setting.SetGlobalIPWhitelistEnabled(original.Enabled)
	})
	require.NoError(t, setting.UpdateGlobalIPWhitelistCIDRs(`["203.0.113.8"]`))
	setting.SetGlobalIPWhitelistEnabled(true)

	gin.SetMode(gin.TestMode)
	router := gin.New()
	require.NoError(t, router.SetTrustedProxies(nil))
	router.Use(GlobalIPWhitelist())
	router.GET("/private", func(c *gin.Context) { c.Status(http.StatusNoContent) })
	router.GET("/api/status", func(c *gin.Context) { c.Status(http.StatusNoContent) })
	router.GET("/api/livez", func(c *gin.Context) { c.Status(http.StatusNoContent) })

	request := func(path, remoteAddr string) int {
		recorder := httptest.NewRecorder()
		req := httptest.NewRequest(http.MethodGet, path, nil)
		req.RemoteAddr = remoteAddr
		router.ServeHTTP(recorder, req)
		return recorder.Code
	}

	assert.Equal(t, http.StatusNoContent, request("/private", "203.0.113.8:1234"))
	assert.Equal(t, http.StatusForbidden, request("/private", "203.0.113.9:1234"))
	assert.Equal(t, http.StatusNoContent, request("/api/status", "203.0.113.9:1234"))
	assert.Equal(t, http.StatusNoContent, request("/api/livez", "203.0.113.9:1234"))
}
