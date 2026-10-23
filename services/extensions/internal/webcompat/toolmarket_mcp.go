// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package webcompat

import (
	"github.com/TokenNotIncluded/lmm-api/services/extensions/internal/modules/toolmarket"
	"net/http"
)

type ToolMarketLegacyMCP interface {
	LegacyMarketMCPHandler(toolmarket.LegacyMarketMCPOptions) http.Handler
}

// RegisterToolMarketLegacyMCP mounts the resource independently of browser
// Session routes. Identity comes only from its resource bearer token.
func RegisterToolMarketLegacyMCP(mux *http.ServeMux, service ToolMarketLegacyMCP, opts toolmarket.LegacyMarketMCPOptions) {
	if mux == nil || service == nil {
		return
	}
	handler := service.LegacyMarketMCPHandler(opts)
	mux.Handle("/mcp/market", handler)
	mux.Handle("/mcp/market/", handler)
}
