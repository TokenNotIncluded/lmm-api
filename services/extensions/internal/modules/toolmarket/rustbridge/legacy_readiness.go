// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package rustbridge

import (
	"context"
	market "github.com/TokenNotIncluded/lmm-api/services/extensions/internal/modules/toolmarket"
	"sync"
)

type legacyReadinessState struct {
	mu         sync.RWMutex
	generation uint64
	ack        market.LegacyCapabilities
}

var _ market.CoreLegacyReadiness = (*Adapter)(nil)

func (a *Adapter) LegacyReady(ctx context.Context, requireOAuth bool) error {
	if a == nil {
		return market.ErrUnavailable
	}
	a.legacyReady.mu.Lock()
	a.legacyReady.generation++
	generation := a.legacyReady.generation
	a.legacyReady.ack = market.LegacyCapabilities{}
	a.legacyReady.mu.Unlock()
	var ack market.LegacyCapabilities
	if e := a.requestVersion(ctx, 2, "capabilities", "", struct{}{}, &ack); e != nil {
		return e
	}
	if e := market.ValidateLegacyCapabilities(ack, requireOAuth); e != nil {
		return e
	}
	a.legacyReady.mu.Lock()
	defer a.legacyReady.mu.Unlock()
	if generation != a.legacyReady.generation {
		return market.ErrUnavailable
	}
	ack.Features = append([]string(nil), ack.Features...)
	a.legacyReady.ack = ack
	return nil
}
func (a *Adapter) LegacyReadiness() market.LegacyCapabilities {
	if a == nil {
		return market.LegacyCapabilities{}
	}
	a.legacyReady.mu.RLock()
	defer a.legacyReady.mu.RUnlock()
	ack := a.legacyReady.ack
	ack.Features = append([]string(nil), ack.Features...)
	return ack
}
