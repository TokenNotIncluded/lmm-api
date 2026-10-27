// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package webcompat

import (
	"net/http/httptest"
	"testing"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/toolmarket"
)

func TestToolMarketConnectionCheckRequiresAuthorityBeforeReadingCredential(t *testing.T) {
	core, service := &authCore{}, &marketTransportFake{authErr: toolmarket.ErrDenied}
	reader := &marketUnreadBody{}
	request := marketHTTP("POST", "/api/tool-market/connection/check", "", true)
	request.Header.Set("Content-Type", "application/json")
	request.Body = reader
	recorder := httptest.NewRecorder()
	marketMux(t, core, service).ServeHTTP(recorder, request)
	if recorder.Code != 403 || service.permission != "toolmarket.manage" || service.authCalls != 1 || service.businessCalls != 0 || reader.reads != 0 {
		t.Fatal(recorder.Code, service, reader.reads)
	}
}

func TestToolMarketRemoteErrorsDoNotInvalidateLMMSession(t *testing.T) {
	for _, tc := range []struct {
		err    error
		status int
		code   string
	}{
		{toolmarket.ErrRemoteUnauthorized, 502, "TOOL_MARKET_REMOTE_UNAUTHORIZED"},
		{toolmarket.ErrRemoteForbidden, 502, "TOOL_MARKET_REMOTE_FORBIDDEN"},
		{toolmarket.ErrRemoteTimeout, 504, "TOOL_MARKET_REMOTE_TIMEOUT"},
		{toolmarket.ErrUpstream, 502, "TOOL_MARKET_REMOTE_CONNECTION"},
		{toolmarket.ErrAuth, 401, "TOOL_MARKET_LOGIN_REQUIRED"},
		{toolmarket.ErrDenied, 403, "TOOL_MARKET_DENIED"},
	} {
		t.Run(tc.code, func(t *testing.T) {
			recorder := httptest.NewRecorder()
			toolMarketRespond(recorder, nil, tc.err)
			body := marketEnvelope(t, recorder)
			if recorder.Code != tc.status || body["success"] != false || body["code"] != tc.code {
				t.Fatal(recorder.Code, body)
			}
		})
	}
	result := toolmarket.WebConnectionCheck{Provider: "treg", Surface: "team", Authentication: "not_verified", Status: "unauthorized", UpstreamStatus: 401}
	recorder := httptest.NewRecorder()
	toolMarketRespond(recorder, result, nil)
	body := marketEnvelope(t, recorder)
	if recorder.Code != 200 || body["success"] != true || body["data"].(map[string]any)["authentication"] != "not_verified" {
		t.Fatal("observed upstream rejection became an LMM session failure", recorder.Code, body)
	}
}
