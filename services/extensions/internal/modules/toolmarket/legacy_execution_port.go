// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package toolmarket

import "context"

// CoreLegacyCredits is the trusted host CREDIT v2 port. No Go financial counters
// or caller supplied wallet identity participate in these operations.
type CoreLegacyCredits interface {
	ReserveLegacyCredits(context.Context, string, LegacyCreditReserve) (LegacyCreditCall, error)
	DispatchLegacyCredits(context.Context, string, LegacyCreditID) (LegacyCreditCall, error)
	FinishLegacyCredits(context.Context, LegacyCreditFinish) (LegacyCreditCall, error)
	UnknownLegacyCredits(context.Context, LegacyCreditID) (LegacyCreditCall, error)
	ReleaseLegacyCredits(context.Context, LegacyCreditRelease) (LegacyCreditCall, error)
	QueryLegacyCredits(context.Context, string, LegacyCreditID) (LegacyCreditCall, error)
}
type LegacyCreditReserve struct {
	Owner       Principal `json:"owner"`
	ClientID    string    `json:"client_id"`
	RequestKey  string    `json:"request_key"`
	QuoteID     string    `json:"quote_id"`
	GrantID     string    `json:"grant_id"`
	ToolID      string    `json:"tool_id"`
	VersionID   string    `json:"version_id"`
	InputDigest string    `json:"input_digest"`
	ResolveBy   int64     `json:"resolve_by"`
}
type LegacyCreditID struct {
	Owner      Principal `json:"owner"`
	CallID     string    `json:"call_id"`
	DispatchID string    `json:"dispatch_id,omitempty"`
}
type LegacyCreditUsage struct {
	Source        string           `json:"source"`
	CallID        string           `json:"call_id"`
	VersionID     string           `json:"version_id"`
	ToolID        string           `json:"tool_id"`
	InputDigest   string           `json:"input_digest"`
	PricingDigest string           `json:"pricing_digest"`
	ResultDigest  string           `json:"result_digest"`
	Quantities    map[string]int64 `json:"quantities"`
}

// ProofKind is host-side routing derived from a durable price snapshot. It is
// never accepted from or serialized into a user/provider request body.
type LegacyProofKind uint8

const (
	LegacyProofOrdinary LegacyProofKind = iota
	LegacyProofFixedWitness
)

type LegacyFixedWitness interface {
	FixedWitnessAvailable() bool
	AdmitFixedWitness(context.Context, string) error
}
type LegacyCreditFinish struct {
	ProofKind LegacyProofKind `json:"-"`
	RawResult string          `json:"raw_result,omitempty"`
	LegacyCreditID
	Success      bool               `json:"success"`
	ResultDigest string             `json:"result_digest"`
	Usage        *LegacyCreditUsage `json:"usage,omitempty"`
}
type LegacyCreditRelease struct {
	RawResult string `json:"raw_result,omitempty"`
	LegacyCreditID
	Reason         string `json:"reason"`
	EvidenceDigest string `json:"evidence_digest"`
}
type LegacyCreditCall struct {
	ID              string       `json:"call_id"`
	Owner           Principal    `json:"owner"`
	ClientID        string       `json:"client_id"`
	RequestKey      string       `json:"request_key"`
	QuoteID         string       `json:"quote_id"`
	GrantID         string       `json:"grant_id"`
	ToolID          string       `json:"tool_id"`
	VersionID       string       `json:"version_id"`
	InputDigest     string       `json:"input_digest"`
	PricingDigest   string       `json:"pricing_digest"`
	CreatedAt       int64        `json:"created_at"`
	ResolveBy       int64        `json:"resolve_by"`
	State           string       `json:"state"`
	Reservation     string       `json:"reservation"`
	Held            CreditAmount `json:"held_quota"`
	Charged         CreditAmount `json:"charged_quota"`
	Fee             CreditAmount `json:"fee_quota"`
	Seller          CreditAmount `json:"seller_quota"`
	Refunded        CreditAmount `json:"refunded_quota"`
	DispatchID      string       `json:"dispatch_id"`
	DispatchClaimed bool         `json:"dispatch_won"`
	ResultDigest    string       `json:"result_digest"`
}

func legacyCallMatches(c LegacyCreditCall, r LegacyExecution) bool {
	if r.SettlementPricingDigest != "" && c.PricingDigest != r.SettlementPricingDigest {
		return false
	}
	if r.ResultDigest != "" && c.State == "settled" && c.ResultDigest != legacyAuthorityDigest(r) {
		return false
	}
	switch c.State {
	case "reserved", "running", "unknown", "settled", "released":
	default:
		return false
	}
	if c.ResolveBy <= 0 || !c.Fee.Valid() || !c.Seller.Valid() || !c.Refunded.Valid() {
		return false
	}
	return validID.MatchString(c.ID) && c.Owner == r.Owner && c.ClientID == r.Input.ClientID && c.RequestKey == r.Input.RequestKey && c.QuoteID == r.QuoteID && c.GrantID == r.Input.GrantID && c.ToolID == r.Input.ToolID && c.VersionID == r.Input.VersionID && c.InputDigest == r.InputDigest && c.Held.Valid() && c.Charged.Valid() && c.Charged <= c.Held
}
