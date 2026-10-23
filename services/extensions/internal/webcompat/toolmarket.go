// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
// Routes and DTOs follow main 6efcb6fb0f6d apps/api-go/router/tool_market_router.go.
package webcompat

import (
	"context"
	"encoding/json"
	"errors"
	"io"
	"mime"
	"net"
	"net/http"
	"net/url"
	"strconv"
	"strings"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/toolmarket"
)

type ToolMarketService interface {
	PublicWebConfig(context.Context) (map[string]any, error)
	PublicWebCatalogue(context.Context, string, string, toolmarket.WebQuery) ([]toolmarket.WebSummary, error)
	PublicWebDetail(context.Context, string, string, string) (toolmarket.WebDetail, error)
	WebAuthorize(context.Context, string, string, string) (int64, error)
	LegacyWeb(context.Context, string, string, toolmarket.WebRequest) (any, error)
}

// ToolMarketRuntime is optional: the native module owns its own closed DTOs,
// USD quote units, OAuth cookie binding and MCP authorization.
type ToolMarketRuntime interface {
	ToolMarketRuntimeHandler() http.Handler
}

type toolMarketRoute struct {
	pattern, action string
	body            bool
	page            bool
}

// RegisterToolMarket is optional host wiring. Public reads work without a Core
// session; all personalized reads and writes verify a live Rust session AND an
// explicit Rust action before reading a request body or module records.
func RegisterToolMarket(mux *http.ServeMux, auth *Authenticator, service ToolMarketService) {
	actor := func(w http.ResponseWriter, r *http.Request, required bool) (string, string, bool) {
		present := false
		for key := range r.Header {
			if strings.EqualFold(key, "Authorization") {
				present = true
			}
		}
		if !required && !present {
			return "", "", true
		}
		self, err := auth.Verify(r.Context(), r)
		if err != nil {
			toolMarketRespond(w, nil, toolMarketAuthError(err))
			return "", "", false
		}
		var credential string
		for name, values := range r.Header {
			if strings.EqualFold(name, "Authorization") {
				credential = strings.TrimPrefix(values[0], "Bearer ")
			}
		}
		return credential, strconv.FormatInt(self.GetUserId(), 10), true
	}
	public := func(fn func(http.ResponseWriter, *http.Request, string, string)) http.HandlerFunc {
		return func(w http.ResponseWriter, r *http.Request) {
			credential, account, ok := actor(w, r, false)
			if !ok {
				return
			}
			if store.Missing(service) {
				toolMarketRespond(w, nil, toolmarket.ErrUnavailable)
				return
			}
			fn(w, r, credential, account)
		}
	}
	mux.HandleFunc("GET /api/tool-market/config", public(func(w http.ResponseWriter, r *http.Request, credential, account string) {
		if _, err := toolMarketQuery(r); err != nil {
			toolMarketRespond(w, nil, err)
			return
		}
		if credential != "" {
			if _, err := service.WebAuthorize(r.Context(), credential, account, "toolmarket.read"); err != nil {
				toolMarketRespond(w, nil, err)
				return
			}
		}
		value, err := service.PublicWebConfig(r.Context())
		toolMarketRespond(w, value, err)
	}))
	mux.HandleFunc("GET /api/tool-market", public(func(w http.ResponseWriter, r *http.Request, credential, account string) {
		query, err := toolMarketQuery(r)
		if err != nil {
			toolMarketRespond(w, nil, err)
			return
		}
		offset, limit, err := toolMarketPage(query)
		if err != nil {
			toolMarketRespond(w, nil, err)
			return
		}
		for key := range query {
			if key != "q" && key != "offset" && key != "limit" && key != "execution_type" {
				toolMarketRespond(w, nil, toolmarket.ErrInvalid)
				return
			}
		}
		rows, err := service.PublicWebCatalogue(r.Context(), credential, account, toolmarket.WebQuery{Text: query.Get("q"), ExecutionType: query.Get("execution_type"), Offset: offset, Limit: limit})
		if rows == nil && err == nil {
			rows = []toolmarket.WebSummary{}
		}
		toolMarketRespond(w, rows, err)
	}))
	mux.HandleFunc("GET /api/tool-market/services/{id}", public(func(w http.ResponseWriter, r *http.Request, credential, account string) {
		query, err := toolMarketQuery(r)
		if err != nil || len(query) != 0 {
			toolMarketRespond(w, nil, toolmarket.ErrInvalid)
			return
		}
		value, err := service.PublicWebDetail(r.Context(), credential, account, r.PathValue("id"))
		toolMarketRespond(w, value, err)
	}))
	routes := []toolMarketRoute{
		{"GET /api/tool-market/services/{id}/credentials", "credentials", false, false},
		{"PUT /api/tool-market/services/{id}/credentials", "credential.configure", true, false},
		{"POST /api/tool-market/services", "save", true, false},
		{"PUT /api/tool-market/services/{id}/draft", "save", true, false},
		{"GET /api/tool-market/services/{id}/draft", "draft", false, false},
		{"POST /api/tool-market/services/{id}/validate", "validate", false, false},
		{"POST /api/tool-market/services/{id}/submit", "submit", true, false},
		{"POST /api/tool-market/services/{id}/activate", "activate", true, false},
		{"DELETE /api/tool-market/services/{id}", "delete", false, false},
		{"PUT /api/tool-market/services/{id}/paused", "pause", true, false},
		{"PUT /api/tool-market/services/{id}/favorite", "favorite", true, false},
		{"PUT /api/tool-market/installations", "install", true, false},
		{"GET /api/tool-market/mine/{kind}", "mine", false, true},
		{"GET /api/tool-market/reviews", "reviews", false, true},
		{"GET /api/tool-market/services/{id}/review", "review_detail", false, false},
		{"POST /api/tool-market/services/{id}/review", "review", true, false},
		{"POST /api/tool-market/inspect", "inspect", true, false},
		{"GET /api/tool-market/calls", "calls", false, true},
		{"GET /api/tool-market/calls/{id}/result", "call.result", false, false},
		{"GET /api/tool-market/executions/by-request/{id}", "call.lookup", false, false},
		{"POST /api/tool-market/calls/{id}/resume", "call.resume", true, false},
		{"POST /api/tool-market/calls/{id}/report", "report", true, false},
		{"GET /api/tool-market/reports", "reports", false, true},
		{"POST /api/tool-market/reports/{id}/review", "report.review", true, false},
		{"GET /api/tool-market/income", "income", false, true},
		{"GET /api/tool-market/meta-delegations/{kind}/{id}", "delegation.get", false, false},
		{"GET /api/tool-market/meta-delegations/oauth-clients", "delegation.clients", false, true},
		{"PUT /api/tool-market/meta-delegations/{kind}/{id}", "delegation.set", true, false},
		{"POST /api/tool-market/invoke", "invoke", true, false},
		{"POST /api/tool-market/drawing/invoke", "invoke", true, false},
		{"DELETE /api/tool-market/grants/{id}", "grant.revoke", false, false},
		{"DELETE /api/tool-market/grants/{id}/record", "grant.hide", false, false},
		{"POST /api/tool-market/tokens", "token", true, false},
		{"DELETE /api/tool-market/tokens/{id}", "token.revoke", false, false},
		{"DELETE /api/tool-market/tokens/{id}/record", "token.hide", false, false},
		{"POST /api/tool-market/clients/disconnect", "client.disconnect", true, false},
		{"POST /api/tool-market/clients/remove", "client.remove", true, false},
		{"POST /api/tool-market/grants", "grant", true, false},
		{"PUT /api/tool-market/budgets", "budget", true, false},
		{"PUT /api/tool-market/config", "configure", true, false},
	}
	for _, route := range routes {
		mux.HandleFunc(route.pattern, func(w http.ResponseWriter, r *http.Request) {
			credential, account, ok := actor(w, r, true)
			if !ok {
				return
			}
			if store.Missing(service) {
				toolMarketRespond(w, nil, toolmarket.ErrUnavailable)
				return
			}
			if _, err := service.WebAuthorize(r.Context(), credential, account, toolmarket.WebPermission(route.action)); err != nil {
				toolMarketRespond(w, nil, err)
				return
			}
			query, err := toolMarketQuery(r)
			if err != nil {
				toolMarketRespond(w, nil, err)
				return
			}
			request := toolmarket.WebRequest{Action: route.action, ID: r.PathValue("id"), Kind: r.PathValue("kind"), Offset: 0, Limit: 20}
			if peer, _, err := net.SplitHostPort(r.RemoteAddr); err == nil {
				request.ActualPeerIP = peer
			}
			if route.page {
				for key := range query {
					if key != "offset" && key != "limit" {
						toolMarketRespond(w, nil, toolmarket.ErrInvalid)
						return
					}
				}
				request.Offset, request.Limit, err = toolMarketPage(query)
			} else if route.action == "credentials" {
				if len(query) != 1 || query.Get("version_id") == "" {
					err = toolmarket.ErrInvalid
				} else {
					request.VersionID = query.Get("version_id")
				}
			} else if len(query) != 0 {
				err = toolmarket.ErrInvalid
			}
			if err != nil {
				toolMarketRespond(w, nil, err)
				return
			}
			var operations []string
			for key, values := range r.Header {
				if strings.EqualFold(key, "Idempotency-Key") {
					operations = append(operations, values...)
				}
			}
			if len(operations) > 1 || len(operations) == 1 && (len(operations[0]) == 0 || len(operations[0]) > 80) {
				toolMarketRespond(w, nil, toolmarket.ErrInvalid)
				return
			}
			if len(operations) == 1 {
				request.OperationID = operations[0]
			}
			if route.body {
				if route.pattern == "POST /api/tool-market/drawing/invoke" {
					select {
					case toolMarketDrawingUploadSlots <- struct{}{}:
						defer func() { <-toolMarketDrawingUploadSlots }()
					default:
						toolMarketRespond(w, nil, toolmarket.ErrLimit)
						return
					}
					var uploads []toolmarket.DrawingUpload
					request.Body, uploads, err = toolMarketDrawingBody(w, r)
					if err == nil {
						r = r.WithContext(toolmarket.WithDrawingUploads(r.Context(), uploads))
					}
				} else {
					if route.action == "delegation.set" {
						r.Body = http.MaxBytesReader(w, r.Body, 1024)
					}
					request.Body, err = toolMarketBody(w, r)
				}
				if err != nil {
					toolMarketRespond(w, nil, err)
					return
				}
			}
			value, err := service.LegacyWeb(r.Context(), credential, account, request)
			toolMarketRespond(w, value, err)
		})
	}
	// Runtime APIs retain the native module's typed contract. They do not reinterpret
	// legacy price_quota as USD millionths. Replace all internal identity headers
	// with the freshly verified Web actor before invoking the native handler.
	if runtime, ok := service.(ToolMarketRuntime); ok && !store.Missing(runtime) {
		mux.HandleFunc("/api/tool-market/runtime/", func(w http.ResponseWriter, r *http.Request) {
			credential, account, ok := actor(w, r, true)
			if !ok {
				return
			}
			if _, err := service.WebAuthorize(r.Context(), credential, account, "toolmarket.read"); err != nil {
				toolMarketRespond(w, nil, err)
				return
			}
			handler := runtime.ToolMarketRuntimeHandler()
			if store.Missing(handler) {
				toolMarketRespond(w, nil, toolmarket.ErrUnavailable)
				return
			}
			request := r.Clone(r.Context())
			request.Header = r.Header.Clone()
			for key := range request.Header {
				if strings.HasPrefix(strings.ToLower(key), "x-lmm-") {
					delete(request.Header, key)
				}
			}
			request.Header.Set("X-LMM-User-Credential", credential)
			request.Header.Set("X-LMM-Account-ID", account)
			request.URL.Path = strings.TrimPrefix(r.URL.Path, "/api/tool-market/runtime")
			request.URL.RawPath = ""
			handler.ServeHTTP(w, request)
		})
	}
	// Explicitly reject unimplemented OAuth, metatool, credentials, paid call
	// history, grants and budgets. An unavailable module cannot fabricate lists.
	mux.HandleFunc("/api/tool-market/", func(w http.ResponseWriter, r *http.Request) {
		credential, account, ok := actor(w, r, true)
		if !ok {
			return
		}
		if !store.Missing(service) {
			if _, err := service.WebAuthorize(r.Context(), credential, account, "toolmarket.read"); err != nil {
				toolMarketRespond(w, nil, err)
				return
			}
		}
		toolMarketRespond(w, nil, toolmarket.ErrUnavailable)
	})
}
func toolMarketQuery(r *http.Request) (url.Values, error) {
	query, err := url.ParseQuery(r.URL.RawQuery)
	if err != nil {
		return nil, toolmarket.ErrInvalid
	}
	for _, values := range query {
		if len(values) != 1 {
			return nil, toolmarket.ErrInvalid
		}
	}
	return query, nil
}
func toolMarketPage(query url.Values) (int, int, error) {
	offset, limit := 0, 20
	var err error
	if value, ok := query["offset"]; ok {
		offset, err = strconv.Atoi(value[0])
		if err != nil {
			return 0, 0, toolmarket.ErrInvalid
		}
	}
	if value, ok := query["limit"]; ok {
		limit, err = strconv.Atoi(value[0])
		if err != nil {
			return 0, 0, toolmarket.ErrInvalid
		}
	}
	if offset < 0 || offset > 10000 || limit < 1 || limit > 100 {
		return 0, 0, toolmarket.ErrInvalid
	}
	return offset, limit, nil
}
func toolMarketBody(w http.ResponseWriter, r *http.Request) (json.RawMessage, error) {
	media, _, err := mime.ParseMediaType(r.Header.Get("Content-Type"))
	if err != nil || media != "application/json" {
		return nil, toolmarket.ErrInvalid
	}
	body, err := io.ReadAll(http.MaxBytesReader(w, r.Body, 256<<10))
	if err != nil || len(body) == 0 {
		return nil, toolmarket.ErrInvalid
	}
	return body, nil
}
func toolMarketAuthError(err error) error {
	switch {
	case errors.Is(err, store.ErrUnauthorized):
		return toolmarket.ErrAuth
	case errors.Is(err, store.ErrForbidden):
		return toolmarket.ErrDenied
	case errors.Is(err, store.ErrInvalid):
		return toolmarket.ErrInvalid
	default:
		return toolmarket.ErrUnavailable
	}
}
func toolMarketRespond(w http.ResponseWriter, value any, err error) {
	code := http.StatusOK
	body := map[string]any{"success": true, "message": "", "data": value}
	if err != nil {
		label, message := "TOOL_MARKET_UNAVAILABLE", "Tool market operation could not be completed."
		code = http.StatusServiceUnavailable
		switch {
		case errors.Is(err, toolmarket.ErrInvalid):
			code, label = http.StatusUnprocessableEntity, "TOOL_MARKET_INVALID_INPUT"
		case errors.Is(err, toolmarket.ErrAuth):
			code, label = http.StatusUnauthorized, "TOOL_MARKET_LOGIN_REQUIRED"
		case errors.Is(err, toolmarket.ErrDenied):
			code, label = http.StatusForbidden, "TOOL_MARKET_DENIED"
		case errors.Is(err, toolmarket.ErrMissing):
			code, label = http.StatusNotFound, "TOOL_MARKET_NOT_FOUND"
		case errors.Is(err, toolmarket.ErrConflict):
			code, label = http.StatusConflict, "TOOL_MARKET_CONFLICT"
		case errors.Is(err, toolmarket.ErrPending):
			code, label = http.StatusConflict, "TOOL_MARKET_EXECUTION_PENDING"
		case errors.Is(err, toolmarket.ErrConsent):
			code, label = http.StatusConflict, "TOOL_MARKET_CONSENT_REQUIRED"
		case errors.Is(err, toolmarket.ErrPrice):
			code, label = http.StatusUnprocessableEntity, "TOOL_MARKET_PROVIDER_PRICE"
		case errors.Is(err, toolmarket.ErrFunds):
			label = "TOOL_MARKET_FUNDS_UNAVAILABLE"
		case errors.Is(err, toolmarket.ErrLimit):
			code, label = http.StatusTooManyRequests, "TOOL_MARKET_BUSY"
			w.Header().Set("Retry-After", "60")
		case errors.Is(err, toolmarket.ErrUpstream):
			code, label = http.StatusBadGateway, "TOOL_MARKET_REMOTE_CONNECTION"
		}
		body = map[string]any{"success": false, "code": label, "message": message}
		if errors.Is(err, toolmarket.ErrPending) && value != nil {
			body["data"] = value
		}
	}
	encoded, marshalErr := json.Marshal(body)
	if marshalErr != nil {
		code = http.StatusServiceUnavailable
		encoded = []byte(`{"success":false,"code":"TOOL_MARKET_UNAVAILABLE","message":"Tool market operation could not be completed."}`)
	}
	w.Header().Set("Content-Type", "application/json; charset=utf-8")
	w.Header().Set("Cache-Control", "no-store")
	w.Header().Set("X-Content-Type-Options", "nosniff")
	w.WriteHeader(code)
	_, _ = w.Write(append(encoded, '\n'))
}
