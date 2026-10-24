// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package webcompat

import (
	"net/http"
	"strings"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/operations"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
)

// Mount keeps interactive Web traffic separate from the service-token module
// API while sharing its admission/drain lifecycle. It never upgrades the host
// credential into user authority.
type AdditionalServices struct {
	Builtins          BuiltinHTTP
	ToolMarketMCP     http.Handler
	Assistant         AssistantHistory
	AssistantChat     AssistantChat
	AssistantServices http.Handler
	ToolMarket        ToolMarketService
	Operations        *operations.Service
	Payments          PaymentStatusReader
	Plugins           PluginController
}

// @mark extensions.admission role=consumer
func Mount(host *modules.Host, auth *Authenticator, catalogue StoreService, support SupportService, optional ...AdditionalServices) http.Handler {
	mux := http.NewServeMux()
	RegisterStore(mux, auth, catalogue)
	RegisterSupport(mux, auth, support)
	var operationsService *operations.Service
	var paymentReader PaymentStatusReader
	if len(optional) == 1 {
		RegisterBuiltinHTTP(mux, optional[0].Builtins)
		if !store.Missing(optional[0].ToolMarketMCP) {
			mux.Handle("/mcp/market", optional[0].ToolMarketMCP)
			mux.Handle("/mcp/market/", optional[0].ToolMarketMCP)
		}
		operationsService = optional[0].Operations
		paymentReader = optional[0].Payments
		if !store.Missing(optional[0].Assistant) {
			RegisterAssistant(mux, auth, optional[0].Assistant, optional[0].AssistantChat)
		}
		if !store.Missing(optional[0].AssistantServices) {
			handler := optional[0].AssistantServices
			mux.Handle("/api/assistant/services/", http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				if _, err := auth.Verify(r.Context(), r); err != nil {
					Respond(w, nil, err)
					return
				}
				handler.ServeHTTP(w, r)
			}))
		}
		if !store.Missing(optional[0].ToolMarket) {
			RegisterToolMarket(mux, auth, optional[0].ToolMarket)
		}
		if !store.Missing(optional[0].Plugins) {
			RegisterPlugins(mux, auth, optional[0].Plugins)
		}
	}
	RegisterOperations(mux, auth, operationsService)
	RegisterPayments(mux, auth, paymentReader)
	// Unassembled old routes are explicit failures, never a successful empty DTO.
	mux.HandleFunc("/api/store/", func(w http.ResponseWriter, r *http.Request) {
		if _, err := auth.Verify(r.Context(), r); err != nil {
			Respond(w, nil, err)
			return
		}
		Respond(w, nil, store.ErrUnavailable)
	})
	return OperationsGuard(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if !builtinHTTPPath(r.URL.Path) && r.URL.Path != "/mcp/market" && r.URL.Path != "/mcp/market/" && !strings.HasPrefix(r.URL.Path, "/api/store/") && !strings.HasPrefix(r.URL.Path, "/api/assistant/") && r.URL.Path != "/api/tool-market" && !strings.HasPrefix(r.URL.Path, "/api/tool-market/") && !strings.HasPrefix(r.URL.Path, "/api/user/auth/store-claim/") && !strings.HasPrefix(r.URL.Path, "/api/user/payments/") && !pluginHTTPPath(r.URL.Path) &&
			!strings.HasPrefix(r.URL.Path, "/api/release-notes/") && r.URL.Path != "/api/scripts" &&
			!strings.HasPrefix(r.URL.Path, "/api/scripts/") && !strings.HasPrefix(r.URL.Path, "/scripts/") {
			host.ServeHTTP(w, r)
			return
		}
		release, err := host.TryAcquire()
		if err != nil {
			w.Header().Set("Retry-After", "1")
			Respond(w, nil, err)
			return
		}
		defer release()
		if len(optional) == 1 {
			if plugin := webPluginName(r.URL.Path); plugin != "" && !store.Missing(optional[0].Plugins) && !optional[0].Plugins.IsEnabled(plugin) {
				Respond(w, nil, store.ErrUnavailable)
				return
			}
		}
		mux.ServeHTTP(w, r)
	}))
}
