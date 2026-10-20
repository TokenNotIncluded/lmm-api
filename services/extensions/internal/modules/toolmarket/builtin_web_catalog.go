// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package toolmarket

// Derived code records are overlaid on a transaction's private copy. Opening or
// reading a module does not initialize/migrate its database. A code/schema change
// points to a new immutable version, so prior grants cannot authorize it.
func overlayBuiltinCatalog(d *database) {
	w := webState(d)
	for _, definition := range CompiledBuiltinDefinitions() {
		tools := make([]WebTool, 0, len(definition.Tools))
		remote := map[string]string{}
		for _, entry := range definition.Tools {
			tool := entry.WebTool
			tool.PriceQuota = 0
			tool.ProviderPricing = nil
			tool.BillingMode = ""
			tool.BillingRules = nil
			tool.InputTokenPriceQuota = 0
			tool.MaxInputTokens = 0
			tool.VersionID = definition.VersionID
			tools = append(tools, tool)
			remote[tool.ToolID] = entry.InputSchemaDigest
		}
		w.Services[definition.ServiceID] = WebServiceRecord{ID: definition.ServiceID, OwnerID: 0, LiveVersionID: definition.VersionID, Status: "published"}
		w.Versions[definition.VersionID] = webSnapshot{Version: WebVersion{ID: definition.VersionID, ServiceID: definition.ServiceID, Status: "published", Name: definition.Name, Description: definition.Description, ExecutionType: "builtin", Visibility: "public", Digest: definition.Digest}, Tools: tools, Digest: definition.Digest, ValidationDigest: definition.Digest, RemoteDigests: remote}
	}
}
