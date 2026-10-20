// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package composition

import (
	"reflect"
	"testing"
)

func TestStorageBundleNamespacesAndGrantsAreClosed(t *testing.T) {
	for _, component := range []string{"store", "support", "assistant", "payments", "operations", "toolmarket"} {
		plain, err := storagePolicyFor(component, defaultStorageBundle)
		if err != nil || !reflect.DeepEqual(plain.namespaces, []string{"lmm_" + component}) || !reflect.DeepEqual(plain.tables, storageTables(component)) || len(plain.forbidden) != 0 {
			t.Fatal("plain component acquired bundled access", component, err)
		}
		if component != "toolmarket" {
			if _, err := storagePolicyFor(component, toolMarketBuiltinsBundle); err == nil {
				t.Fatal("another component acquired builtin namespaces", component)
			}
		}
		for _, bundle := range []storageBundle{2, 255} {
			if _, err := storagePolicyFor(component, bundle); err == nil {
				t.Fatal("unknown bundle accepted", component, bundle)
			}
		}
	}
	if _, err := storagePolicyFor("toolmarket-builtins", toolMarketBuiltinsBundle); err == nil {
		t.Fatal("unknown component accepted")
	}
	got, err := storagePolicyFor("toolmarket", toolMarketBuiltinsBundle)
	if err != nil {
		t.Fatal(err)
	}
	want := storagePolicy{
		namespaces: []string{"lmm_toolmarket", "lmm_bounties", "extension_wallet"},
		tables: []storageTable{
			{"lmm_toolmarket.snapshot", []string{"SELECT", "UPDATE"}},
			{"lmm_bounties.contract", []string{"SELECT"}},
			{"lmm_bounties.records", []string{"SELECT", "INSERT", "UPDATE"}},
			{"extension_wallet.transfer_links", []string{"SELECT", "INSERT"}},
		},
		forbidden: []storageTable{
			{"lmm_toolmarket.snapshot", []string{"INSERT", "DELETE", "TRUNCATE", "REFERENCES", "TRIGGER", "MAINTAIN"}},
			{"lmm_bounties.contract", []string{"INSERT", "UPDATE", "DELETE", "TRUNCATE", "REFERENCES", "TRIGGER", "MAINTAIN"}},
			{"lmm_bounties.records", []string{"DELETE", "TRUNCATE", "REFERENCES", "TRIGGER", "MAINTAIN"}},
			{"extension_wallet.transfer_links", []string{"UPDATE", "DELETE", "TRUNCATE", "REFERENCES", "TRIGGER", "MAINTAIN"}},
		},
		forbiddenColumns: []storageTable{
			{"lmm_toolmarket.snapshot", []string{"INSERT", "REFERENCES"}},
			{"lmm_bounties.contract", []string{"INSERT", "UPDATE", "REFERENCES"}},
			{"lmm_bounties.records", []string{"REFERENCES"}},
			{"extension_wallet.transfer_links", []string{"UPDATE", "REFERENCES"}},
		},
	}
	if !reflect.DeepEqual(got, want) {
		t.Fatal("builtin bundle escaped its exact schema/privilege contract", got)
	}
	got.namespaces[0] = "core_ledger"
	again, _ := storagePolicyFor("toolmarket", toolMarketBuiltinsBundle)
	if !reflect.DeepEqual(again, want) {
		t.Fatal("returned policy mutated future storage guards")
	}
}
