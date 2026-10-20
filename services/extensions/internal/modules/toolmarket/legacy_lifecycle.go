// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package toolmarket

import "context"

type LegacyServiceLifecycleRequest struct {
	OperationID string `json:"operation_id"`
	ServiceID   string `json:"service_id"`
	Paused      bool   `json:"paused"`
	Deleted     bool   `json:"deleted"`
}
type LegacyServiceLifecycleReceipt struct {
	LegacyServiceLifecycleRequest
	Suspended bool  `json:"suspended"`
	Revision  int64 `json:"revision"`
	UpdatedBy int64 `json:"updated_by"`
	UpdatedAt int64 `json:"updated_at"`
}
type CoreLegacyLifecycle interface {
	SetLegacyServiceLifecycle(context.Context, string, LegacyServiceLifecycleRequest) (LegacyServiceLifecycleReceipt, error)
}

func (r *LegacyServiceLifecycleReceipt) UnmarshalJSON(raw []byte) error {
	type alias LegacyServiceLifecycleReceipt
	var next alias
	if err := decodeStrict(raw, &next); err != nil {
		return err
	}
	if err := legacyRequiredFields(raw, "operation_id", "service_id", "paused", "deleted", "suspended", "revision", "updated_by", "updated_at"); err != nil {
		return err
	}
	*r = LegacyServiceLifecycleReceipt(next)
	return nil
}

func legacyLifecycleStatus(r LegacyServiceLifecycleReceipt) string {
	if r.Deleted {
		return "deleted"
	}
	if r.Suspended {
		return "suspended"
	}
	if r.Paused {
		return "paused"
	}
	return "published"
}

// Call this again in the SAME local transaction that sets status and records
// CoreLifecycleRevision. A delayed ACK cannot overwrite a newer local revision.
func legacyValidateLifecycleAck(currentRevision int64, currentStatus string, r LegacyServiceLifecycleReceipt) error {
	if currentRevision < 0 || r.Revision < 1 || r.UpdatedBy <= 0 || r.UpdatedAt <= 0 || !validID.MatchString(r.OperationID) || !validID.MatchString(r.ServiceID) || r.Suspended && !r.Paused && !r.Deleted {
		return ErrDenied
	}
	if r.Revision < currentRevision || currentStatus == "deleted" && !r.Deleted {
		return ErrConflict
	}
	if r.Revision == currentRevision && legacyLifecycleStatus(r) != currentStatus {
		return ErrConflict
	}
	return nil
}
func (m *Module) legacySetServiceLifecycle(ctx context.Context, credential, account, operation, serviceID string, paused, deleted bool) (LegacyServiceLifecycleReceipt, error) {
	if !validID.MatchString(operation) || !validID.MatchString(serviceID) {
		return LegacyServiceLifecycleReceipt{}, ErrInvalid
	}
	actor, err := m.WebAuthorize(ctx, credential, account, "toolmarket.read")
	if err != nil {
		return LegacyServiceLifecycleReceipt{}, err
	}
	port, ok := m.authority.(CoreLegacyLifecycle)
	if !ok || missingInterface(port) {
		return LegacyServiceLifecycleReceipt{}, ErrUnavailable
	}
	var captured WebServiceRecord
	err = m.store.view(func(d database) error {
		if d.Web == nil {
			return ErrMissing
		}
		var ok bool
		captured, ok = d.Web.Services[serviceID]
		if !ok {
			return ErrMissing
		}
		if captured.OwnerID <= 0 {
			return ErrDenied
		}
		if captured.Status == "deleted" && !deleted {
			return ErrConflict
		}
		if !deleted && captured.LiveVersionID == "" {
			return ErrConflict
		}
		if actor == captured.OwnerID && captured.Status == "suspended" && !paused && !deleted {
			return ErrDenied
		}
		return nil
	})
	if err != nil {
		return LegacyServiceLifecycleReceipt{}, err
	}
	permission := "toolmarket.manage"
	if actor != captured.OwnerID {
		permission = "toolmarket.review"
	}
	if current, err := m.WebAuthorize(ctx, credential, account, permission); err != nil {
		return LegacyServiceLifecycleReceipt{}, err
	} else if current != actor {
		return LegacyServiceLifecycleReceipt{}, ErrDenied
	}
	req := LegacyServiceLifecycleRequest{OperationID: operation, ServiceID: serviceID, Paused: paused, Deleted: deleted}
	out, err := port.SetLegacyServiceLifecycle(ctx, credential, req)
	if err != nil {
		return LegacyServiceLifecycleReceipt{}, err
	}
	if out.LegacyServiceLifecycleRequest != req || out.UpdatedBy != actor || out.UpdatedAt > m.now().Unix() {
		return LegacyServiceLifecycleReceipt{}, ErrDenied
	}
	if !deleted && actor != captured.OwnerID && out.Suspended != paused {
		return LegacyServiceLifecycleReceipt{}, ErrDenied
	}
	if !deleted && actor == captured.OwnerID && captured.Status == "suspended" && !out.Suspended {
		return LegacyServiceLifecycleReceipt{}, ErrDenied
	}
	if current, err := m.WebAuthorize(ctx, credential, account, permission); err != nil {
		return LegacyServiceLifecycleReceipt{}, err
	} else if current != actor {
		return LegacyServiceLifecycleReceipt{}, ErrDenied
	}
	err = m.store.view(func(d database) error {
		if d.Web == nil {
			return ErrMissing
		}
		current, ok := d.Web.Services[serviceID]
		if !ok {
			return ErrMissing
		}
		if current.OwnerID != captured.OwnerID {
			return ErrConflict
		}
		return legacyValidateLifecycleAck(current.CoreLifecycleRevision, current.Status, out)
	})
	if err != nil {
		return LegacyServiceLifecycleReceipt{}, err
	}
	// Missing from Core is intentionally not converted to an ACK. A draft may
	// race an in-flight quote registration; only a durable Core tombstone closes
	// that boundary. The caller must not treat Missing as lifecycle success.
	return out, nil
}
