// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package toolmarket

import "context"

func (m *Module) legacyWebLifecycle(ctx context.Context, credential, account string, p Principal, user int64, in WebRequest, paused, deleted bool) (*LegacyServiceLifecycleReceipt, error) {
	if _, ok := m.authority.(CoreLegacyLifecycle); !ok {
		if m.webOnly {
			return nil, nil
		}
		return nil, ErrUnavailable
	}
	var captured WebServiceRecord
	if err := m.store.view(func(d database) error {
		var err error
		captured, err = webOwned(webState(&d), user, in.ID)
		if err != nil {
			return err
		}
		if !deleted && captured.Status != "published" && captured.Status != "paused" {
			return ErrConflict
		}
		return nil
	}); err != nil {
		return nil, err
	}
	operation := in.OperationID
	if operation == "" {
		operation = digest([]any{"legacy-service-lifecycle-v2", p, in.ID, captured.CoreLifecycleRevision, paused, deleted})
	}
	receipt, err := m.legacySetServiceLifecycle(ctx, credential, account, operation, in.ID, paused, deleted)
	if err != nil {
		return nil, err
	}
	return &receipt, nil
}
