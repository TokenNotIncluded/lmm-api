// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package rustbridge

import (
	"context"
	market "github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/toolmarket"
)

func (a *Adapter) ReserveLegacyCredits(ctx context.Context, user string, in market.LegacyCreditReserve) (market.LegacyCreditCall, error) {
	var out market.LegacyCreditCall
	e := a.requestVersion(ctx, 2, "calls/reserve", user, in, &out)
	return out, e
}
func (a *Adapter) DispatchLegacyCredits(ctx context.Context, user string, in market.LegacyCreditID) (market.LegacyCreditCall, error) {
	var out market.LegacyCreditCall
	e := a.requestVersion(ctx, 2, "calls/dispatch", user, struct {
		CallID string `json:"call_id"`
	}{in.CallID}, &out)
	return out, e
}
func (a *Adapter) FinishLegacyCredits(ctx context.Context, in market.LegacyCreditFinish) (market.LegacyCreditCall, error) {
	var out market.LegacyCreditCall
	if in.ProofKind == market.LegacyProofFixedWitness {
		if in.Usage != nil || in.RawResult == "" {
			return out, market.ErrInvalid
		}
		if !a.FixedWitnessAvailable() {
			return out, market.ErrUnavailable
		}
		if !in.Success {
			body := struct {
				CallID         string `json:"call_id"`
				DispatchID     string `json:"dispatch_id"`
				Reason         string `json:"reason"`
				EvidenceDigest string `json:"evidence_digest"`
				RawResult      string `json:"raw_result"`
			}{in.CallID, in.DispatchID, "failed", in.ResultDigest, in.RawResult}
			e := a.fixedWitness.requestLegacyFixedWitnessRelease(ctx, body, &out)
			return out, e
		}
	}
	if !in.Success {
		return a.ReleaseLegacyCredits(ctx, market.LegacyCreditRelease{LegacyCreditID: in.LegacyCreditID, Reason: "failed", EvidenceDigest: in.ResultDigest, RawResult: in.RawResult})
	}
	body := struct {
		CallID       string                    `json:"call_id"`
		DispatchID   string                    `json:"dispatch_id"`
		ResultDigest string                    `json:"result_digest"`
		Usage        *market.LegacyCreditUsage `json:"usage"`
		RawResult    *string                   `json:"raw_result"`
	}{in.CallID, in.DispatchID, in.ResultDigest, in.Usage, nil}
	if in.RawResult != "" {
		body.RawResult = &in.RawResult
	}
	if in.ProofKind == market.LegacyProofFixedWitness {
		if in.Usage != nil || in.RawResult == "" {
			return out, market.ErrInvalid
		}
		if !a.FixedWitnessAvailable() {
			return out, market.ErrUnavailable
		}
		e := a.fixedWitness.requestLegacyFixedWitnessSettlement(ctx, body, &out)
		return out, e
	}
	e := a.requestLegacySettlement(ctx, body, &out)
	return out, e
}
func (a *Adapter) QueryLegacyCredits(ctx context.Context, user string, in market.LegacyCreditID) (market.LegacyCreditCall, error) {
	var out *market.LegacyCreditCall
	e := a.requestVersion(ctx, 2, "calls/query", user, struct {
		CallID string `json:"call_id"`
	}{in.CallID}, &out)
	if e != nil {
		return market.LegacyCreditCall{}, e
	}
	if out == nil {
		return market.LegacyCreditCall{}, market.ErrMissing
	}
	return *out, nil
}
func (a *Adapter) UnknownLegacyCredits(ctx context.Context, in market.LegacyCreditID) (market.LegacyCreditCall, error) {
	var out market.LegacyCreditCall
	e := a.requestVersion(ctx, 2, "calls/unknown", "", struct {
		CallID     string `json:"call_id"`
		DispatchID string `json:"dispatch_id"`
	}{in.CallID, in.DispatchID}, &out)
	return out, e
}
func (a *Adapter) ReleaseLegacyCredits(ctx context.Context, in market.LegacyCreditRelease) (market.LegacyCreditCall, error) {
	var out market.LegacyCreditCall
	body := struct {
		CallID         string  `json:"call_id"`
		DispatchID     string  `json:"dispatch_id"`
		Reason         string  `json:"reason"`
		EvidenceDigest string  `json:"evidence_digest"`
		RawResult      *string `json:"raw_result"`
	}{in.CallID, in.DispatchID, in.Reason, in.EvidenceDigest, nil}
	if in.RawResult != "" {
		body.RawResult = &in.RawResult
	}
	if in.RawResult != "" {
		e := a.requestLegacyResult(ctx, "calls/release", body, &out)
		return out, e
	}
	e := a.requestVersion(ctx, 2, "calls/release", "", body, &out)
	return out, e
}

var _ market.CoreLegacyCredits = (*Adapter)(nil)
