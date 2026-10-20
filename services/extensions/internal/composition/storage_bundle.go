// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package composition

import "errors"

type storageBundle uint8

const (
	defaultStorageBundle storageBundle = iota
	toolMarketBuiltinsBundle
)

type storagePolicy struct {
	namespaces       []string
	tables           []storageTable
	forbidden        []storageTable
	forbiddenColumns []storageTable
}

func storagePolicyFor(component string, bundle storageBundle) (storagePolicy, error) {
	switch component {
	case "store", "support", "assistant", "payments", "operations", "toolmarket":
	default:
		return storagePolicy{}, errors.New("unknown extension storage component")
	}
	if bundle != defaultStorageBundle && (bundle != toolMarketBuiltinsBundle || component != "toolmarket") {
		return storagePolicy{}, errors.New("invalid extension storage bundle")
	}
	policy := storagePolicy{namespaces: []string{"lmm_" + component}, tables: storageTables(component)}
	if bundle == toolMarketBuiltinsBundle {
		policy.namespaces = append(policy.namespaces, "lmm_bounties", "extension_wallet")
		policy.tables = append(policy.tables,
			storageTable{"lmm_bounties.contract", []string{"SELECT"}},
			storageTable{"lmm_bounties.records", []string{"SELECT", "INSERT", "UPDATE"}},
			storageTable{"extension_wallet.transfer_links", []string{"SELECT", "INSERT"}},
		)
		policy.forbidden = []storageTable{
			{"lmm_toolmarket.snapshot", []string{"INSERT", "DELETE", "TRUNCATE", "REFERENCES", "TRIGGER", "MAINTAIN"}},
			{"lmm_bounties.contract", []string{"INSERT", "UPDATE", "DELETE", "TRUNCATE", "REFERENCES", "TRIGGER", "MAINTAIN"}},
			{"lmm_bounties.records", []string{"DELETE", "TRUNCATE", "REFERENCES", "TRIGGER", "MAINTAIN"}},
			{"extension_wallet.transfer_links", []string{"UPDATE", "DELETE", "TRUNCATE", "REFERENCES", "TRIGGER", "MAINTAIN"}},
		}
		policy.forbiddenColumns = []storageTable{
			{"lmm_toolmarket.snapshot", []string{"INSERT", "REFERENCES"}},
			{"lmm_bounties.contract", []string{"INSERT", "UPDATE", "REFERENCES"}},
			{"lmm_bounties.records", []string{"REFERENCES"}},
			{"extension_wallet.transfer_links", []string{"UPDATE", "REFERENCES"}},
		}
	}
	return policy, nil
}
