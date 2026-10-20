// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package rustbridge

import (
	"context"
	market "github.com/TokenNotIncluded/api.lmm.best/extensions/internal/modules/toolmarket"
)

// WithFixedWitness configures a separate installer-owned purpose socket/token.
// Startup readiness is still required; configuration alone never permits spend.
func (a *Adapter) WithFixedWitness(socket string, token []byte) error {
	if a == nil || a.client == nil || a.fixedWitness != nil {
		return market.ErrInvalid
	}
	if string(token) == a.token {
		return market.ErrInvalid
	}
	witness, e := New(socket, token)
	if e != nil {
		return e
	}
	a.fixedWitness = witness
	a.fixedWitnessReviewed = false
	return nil
}
func (a *Adapter) FixedWitnessAvailable() bool {
	return a != nil && a.fixedWitness != nil && a.fixedWitnessReviewed
}

// FixedWitnessReady authenticates the separate purpose bearer and requires
// Core's canonical installed policy before the host can admit a paid fixed call.
func (a *Adapter) FixedWitnessReady(ctx context.Context) error {
	if a == nil || a.fixedWitness == nil {
		return market.ErrUnavailable
	}
	a.fixedWitnessReviewed = false
	var out struct {
		ProtocolMajor int      `json:"protocol_major"`
		Purpose       string   `json:"purpose"`
		Role          string   `json:"role"`
		Features      []string `json:"features"`
	}
	if e := a.fixedWitness.requestVersion(ctx, 2, "fixed-witness/capabilities", "", struct{}{}, &out); e != nil {
		return e
	}
	if out.ProtocolMajor != 2 || out.Purpose != "core_host_fixed_success" || len(out.Role) == 0 || len(out.Role) > 128 {
		return market.ErrUnavailable
	}
	for _, c := range out.Role {
		if c <= 32 || c >= 127 {
			return market.ErrUnavailable
		}
	}
	features := map[string]bool{}
	for _, v := range out.Features {
		if features[v] {
			return market.ErrUnavailable
		}
		features[v] = true
	}
	if !features["admit"] || !features["settle"] || !features["release"] {
		return market.ErrUnavailable
	}
	a.fixedWitnessReviewed = true
	return nil
}
func (a *Adapter) requestLegacyFixedWitnessSettlement(ctx context.Context, in, out any) error {
	return a.requestLegacyResult(ctx, "fixed-witness/settle", in, out)
}

var _ market.LegacyFixedWitness = (*Adapter)(nil)

func (a *Adapter) requestLegacyFixedWitnessRelease(ctx context.Context, in, out any) error {
	return a.requestLegacyResult(ctx, "fixed-witness/release", in, out)
}

// AdmitFixedWitness binds preflight to the actual immutable quote and this
// purpose endpoint's installer role, before any reservation or external call.
func (a *Adapter) AdmitFixedWitness(ctx context.Context, quote string) error {
	if !a.FixedWitnessAvailable() {
		return market.ErrUnavailable
	}
	var out struct {
		QuoteID   string `json:"quote_id"`
		Role      string `json:"role"`
		ProofMode string `json:"proof_mode"`
		Ready     bool   `json:"ready"`
	}
	in := struct {
		QuoteID string `json:"quote_id"`
	}{quote}
	if e := a.fixedWitness.requestVersion(ctx, 2, "fixed-witness/admit", "", in, &out); e != nil {
		return e
	}
	if !out.Ready || out.QuoteID != quote || out.ProofMode != "core_host_fixed_success" || len(out.Role) == 0 || len(out.Role) > 128 {
		return market.ErrUnavailable
	}
	for _, c := range out.Role {
		if c <= 32 || c >= 127 {
			return market.ErrUnavailable
		}
	}
	return nil
}
