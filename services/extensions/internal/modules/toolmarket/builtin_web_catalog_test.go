// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package toolmarket

import "testing"

func TestBuiltinCatalogueOverlayIsImmutableAndVersionBound(t *testing.T) {
	d := emptyDB()
	overlayBuiltinCatalog(&d)
	definitions := CompiledBuiltinDefinitions()
	if len(definitions) != 3 {
		t.Fatalf("compiled services: %d", len(definitions))
	}
	count := 0
	for _, definition := range definitions {
		service := d.Web.Services[definition.ServiceID]
		version := d.Web.Versions[definition.VersionID]
		if service.OwnerID != 0 || service.LiveVersionID != definition.VersionID || !webVisible(service, version, 0) || version.ValidationDigest != definition.Digest {
			t.Fatal("compiled catalogue lost authority or version binding")
		}
		for i, tool := range version.Tools {
			count++
			if tool.VersionID != definition.VersionID || tool.PriceQuota != 0 || tool.ProviderPricing != nil || version.RemoteDigests[tool.ToolID] != definition.Tools[i].InputSchemaDigest {
				t.Fatal("compiled tool binding differs from registry")
			}
		}
		service.OwnerID = 1
		d.Web.Services[definition.ServiceID] = service
	}
	if count != 29 {
		t.Fatalf("compiled tools: %d", count)
	}
	overlayBuiltinCatalog(&d)
	for _, definition := range definitions {
		if d.Web.Services[definition.ServiceID].OwnerID != 0 {
			t.Fatal("persisted records overrode code authority")
		}
	}
}
