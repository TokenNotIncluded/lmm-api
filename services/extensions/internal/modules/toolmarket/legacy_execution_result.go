// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package toolmarket

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"strconv"
	"strings"
)

// LegacyValidateRemoteResult separates definitive failures from uncertain
// business outcomes. A successful HTTP response alone is never sufficient.
func LegacyValidateRemoteResult(raw json.RawMessage, outputSchema string, protected ...string) (json.RawMessage, bool, bool, error) {
	var value map[string]any
	if len(raw) > 2<<20 || strictLegacyJSON(raw, &value, false) != nil || value == nil {
		return legacyFailureResult("Invalid remote result."), false, false, nil
	}
	if legacyResultContains(value, protected) {
		return legacyFailureResult("The remote service returned an unsafe result."), false, false, nil
	}
	if requests, ok := value["inputRequests"]; ok && requests != nil {
		if list, ok := requests.([]any); !ok || len(list) > 0 {
			return nil, false, true, nil
		}
	}
	if s, ok := value["requestState"].(string); ok && s != "" {
		return nil, false, true, nil
	}
	success := true
	if flag, exists := value["isError"]; exists {
		b, ok := flag.(bool)
		if !ok {
			return legacyFailureResult("Invalid remote result."), false, false, nil
		}
		success = !b
	}
	content, _ := value["content"].([]any)
	if rawContent, exists := value["content"]; exists {
		if rawContent == nil || content == nil {
			return legacyFailureResult("Invalid remote content."), false, false, nil
		}
		for _, item := range content {
			part, ok := item.(map[string]any)
			if !ok {
				return legacyFailureResult("Invalid remote content."), false, false, nil
			}
			kind, ok := part["type"].(string)
			if !ok || kind == "" {
				return legacyFailureResult("Invalid remote content."), false, false, nil
			}
			if kind == "text" {
				if _, ok := part["text"].(string); !ok {
					return legacyFailureResult("Invalid remote content."), false, false, nil
				}
			}
		}
	}
	structured := value["structuredContent"]
	if success && len(content) == 0 && structured == nil {
		success = false
	}
	if fields, ok := structured.(map[string]any); ok && success {
		if flag, ok := fields["success"].(bool); ok && !flag {
			success = false
		}
		if state, ok := fields["status"].(string); ok {
			switch strings.ToLower(state) {
			case "pending", "queued", "accepted", "running", "in_progress", "processing":
				return nil, false, true, nil
			case "failed", "failure", "error", "cancelled":
				success = false
			}
		}
	}
	if success && outputSchema != "" {
		encoded, _ := json.Marshal(structured)
		if ValidateLegacyDocument(json.RawMessage(outputSchema), encoded) != nil {
			success = false
		}
	}
	// This validator classifies business failure without authorizing a refund.
	// Paid failure remains held until Core independently verifies its proof.
	return append(json.RawMessage(nil), raw...), success, false, nil
}
func legacyFailureResult(message string) json.RawMessage {
	raw, _ := json.Marshal(map[string]any{"isError": true, "content": []any{map[string]any{"type": "text", "text": message}}})
	return raw
}
func legacyResultContains(value any, protected []string) bool {
	switch v := value.(type) {
	case string:
		for _, s := range protected {
			if s != "" && strings.Contains(v, s) {
				return true
			}
		}
	case map[string]any:
		for k, child := range v {
			if legacyResultContains(k, protected) || legacyResultContains(child, protected) {
				return true
			}
		}
	case []any:
		for _, child := range v {
			if legacyResultContains(child, protected) {
				return true
			}
		}
	}
	return false
}
func (m *Module) legacyFinishRemote(ctx context.Context, port CoreLegacyCredits, r LegacyExecution, tool WebTool, raw json.RawMessage, protected ...string) (LegacyExecution, error) {
	if tool.BillingMode != "" {
		r.Pricing = tool.LegacyPricing()
	}
	clean, success, unknown, e := LegacyValidateRemoteResult(raw, tool.OutputSchema, protected...)
	if e != nil {
		return r, e
	}
	if tool.ProviderPricing != nil && r.ProviderQuote != nil && (success || unknown) {
		pending, valid, runID := legacyProviderBusinessResult(*tool.ProviderPricing, *r.ProviderQuote, raw)
		if pending {
			bound := r
			if legacyValidProviderRunID(runID) {
				var err error
				bound, err = m.legacyBindProviderRun(r, runID)
				if err != nil {
					return r, err
				}
			}
			_, _ = port.UnknownLegacyCredits(ctx, LegacyCreditID{Owner: r.Owner, CallID: r.Core.ID, DispatchID: r.Core.DispatchID})
			return bound, ErrPending
		}
		if !valid {
			success = false
			unknown = false
		}

	}
	if tool.ProviderPricing != nil && tool.ProviderPricing.Provider == "monid" && !unknown {
		clean = legacyMonidPublicResult(clean)
	}
	if unknown {
		_, _ = port.UnknownLegacyCredits(ctx, LegacyCreditID{Owner: r.Owner, CallID: r.Core.ID, DispatchID: r.Core.DispatchID})
		return r, ErrPending
	}
	// Quantities, source and cost belong to Core. Preserve the original signed
	// receipt; never require a Go copy of the supplier key or synthesize usage.
	var usage *LegacyCreditUsage
	if success && r.Core.Held == 0 && tool.BillingMode != "" {
		// Unsigned free usage is display metadata only. A signed report never
		// falls back to unsigned parsing, and no Go quantity authorizes money.
		if display, err := ReadLegacyUsage(tool.LegacyPricing(), clean, nil); err == nil {
			usage = &LegacyCreditUsage{Source: display.Source, Quantities: display.Quantities}
		}
	}

	var exact map[string]any
	if strictLegacyJSON(raw, &exact, false) == nil && !legacyResultContains(exact, protected) {
		r.RawResult = append([]byte(nil), raw...)
		r.AuthorityResultDigest, _ = LegacyMeteringResultDigest(raw)
	}
	if usage != nil {
		usage.ResultDigest = legacyAuthorityDigest(r)
	}
	r.Success = success
	r.Result = clean
	r.ResultDigest = digest(clean)
	r.ResultExpires = m.now().Unix() + 3600
	r.Usage = usage
	r.State = "result"
	// Commit delivery before asking Core to settle. Reconciliation reuses this
	// immutable outcome; it never calls the external tool again.
	if e = m.legacySave(r); e != nil {
		return r, ErrPending
	}
	credential := ""
	if len(protected) > 0 {
		credential = protected[0]
	}
	return m.legacyRecoverExecution(ctx, port, credential, r)
}

// The admitted flag is persisted only from the initial exact Core reserve ACK.
// Old typed-usage candidates cannot acquire authority through a replay.
func legacyNeedsRawReceipt(r LegacyExecution) bool {
	if r.ProviderPricing != nil || r.ProviderQuote != nil {
		return true
	}
	if r.Pricing.BillingMode != "" && r.Core.Held > 0 && r.ResultDigest != "" {
		return !r.SupplierReceiptAdmitted || len(r.RawResult) == 0 || r.AuthorityResultDigest == "" || r.Usage != nil
	}
	return r.ResultDigest != "" && !r.Success && r.Core.Held > 0 && (len(r.RawResult) == 0 || r.AuthorityResultDigest == "")
}

func (m *Module) legacyRecoverExecution(ctx context.Context, port CoreLegacyCredits, credential string, r LegacyExecution) (LegacyExecution, error) {
	if legacyNeedsRawReceipt(r) {
		return r, ErrUnavailable
	}
	if r.State == "not_reserved" {
		return r, ErrConflict
	}
	if r.Core.ID == "" {
		receipt, e := port.ReserveLegacyCredits(ctx, credential, LegacyCreditReserve{Owner: r.Owner, ClientID: r.Input.ClientID, RequestKey: r.Input.RequestKey, QuoteID: r.QuoteID, GrantID: r.Input.GrantID, ToolID: r.Input.ToolID, VersionID: r.Input.VersionID, InputDigest: r.InputDigest, ResolveBy: r.ResolveBy})
		if e != nil {
			return r, ErrPending
		}
		if !legacyCallMatches(receipt, r) {
			return r, ErrDenied
		}
		r.Core = receipt
		r.State = "reserved"
		if m.legacySave(r) != nil {
			return r, ErrPending
		}
	}
	var receipt LegacyCreditCall
	var e error
	if r.ResultDigest != "" {
		receipt, e = legacyFinishCore(ctx, port, r)
	} else {
		receipt, e = port.QueryLegacyCredits(ctx, credential, LegacyCreditID{Owner: r.Owner, CallID: r.Core.ID})
	}
	if e != nil && credential != "" {
		// A lost settlement ACK may already have committed or released the hold.
		// Read the original Core call; never manufacture a second financial action.
		receipt, e = port.QueryLegacyCredits(ctx, credential, LegacyCreditID{Owner: r.Owner, CallID: r.Core.ID})
	}
	if e != nil {
		if r.Builtin != nil && r.ResultDigest == "" {
			r.State = "unknown"
		}
		return r, ErrPending
	}
	if !legacyCallMatches(receipt, r) {
		return r, ErrDenied
	}
	if legacySafeUndispatched(r) && receipt.State == "reserved" && receipt.DispatchID == "" && receipt.ResolveBy <= m.now().Unix() {
		receipt, e = port.ReleaseLegacyCredits(ctx, LegacyCreditRelease{LegacyCreditID: LegacyCreditID{Owner: r.Owner, CallID: r.Core.ID, DispatchID: receipt.DispatchID}, Reason: "expired", EvidenceDigest: ""})
		if e != nil {
			return r, ErrPending
		}
		if !legacyCallMatches(receipt, r) || receipt.State != "released" {
			return r, ErrDenied
		}
	}
	r.Core = receipt
	if r.Builtin != nil && r.ResultDigest == "" && receipt.State != "released" {
		return m.legacyQueryBuiltin(ctx, port, credential, r)
	}
	if receipt.State == "settled" || receipt.State == "released" {
		r.State = "done"
	}
	if e = m.legacySave(r); e != nil {
		return r, ErrPending
	}
	out := r
	if out.ResultExpires > 0 && out.ResultExpires <= m.now().Unix() {
		out.Result = nil
	}
	if r.State != "done" {
		return out, ErrPending
	}
	if r.Builtin != nil && r.Builtin.ExternalResult {
		return m.legacyDeliverDrawing(ctx, credential, out)
	}
	return out, nil
}
func (m *Module) LegacyGetExecution(ctx context.Context, credential, account, id string) (LegacyExecution, error) {
	user, e := m.WebAuthorize(ctx, credential, account, "toolmarket.read")
	if e != nil {
		return LegacyExecution{}, e
	}
	port, ok := m.authority.(CoreLegacyCredits)
	if !ok || missingInterface(port) {
		return LegacyExecution{}, ErrUnavailable
	}
	var r LegacyExecution
	e = m.store.view(func(d database) error {
		var exists bool
		r, exists = d.LegacyExecutions[id]
		if !exists || r.Owner.User != strconv.FormatInt(user, 10) || r.Owner.Account != account {
			return ErrMissing
		}
		return nil
	})
	if e != nil {
		return LegacyExecution{}, e
	}
	return m.legacyRecoverExecution(ctx, port, credential, r)
}

// legacySafeUndispatched requires BOTH the durable local no-dispatch boundary
// and an authoritative reserved receipt. A dispatch intent or lost dispatch ACK
// is not negative evidence of remote execution, even after its old deadline.
func legacySafeUndispatched(r LegacyExecution) bool {
	return r.State == "reserved" && r.Core.State == "reserved" && r.Core.DispatchID == "" && r.ResultDigest == ""
}

// LegacyRecoverPending only reconciles retained results or aborts a positively
// known undispatched reservation. UNKNOWN and missing receipts stay held/pending.
func (m *Module) LegacyRecoverPending(ctx context.Context, limit int) (int, error) {
	if limit < 1 || limit > 1000 {
		return 0, ErrInvalid
	}
	port, ok := m.authority.(CoreLegacyCredits)
	if !ok || missingInterface(port) {
		return 0, ErrUnavailable
	}
	records := []LegacyExecution{}
	if e := m.store.view(func(d database) error {
		for _, r := range d.LegacyExecutions {
			if !legacyNeedsRawReceipt(r) && r.State != "done" && r.Core.ID != "" && (r.ResultDigest != "" || (legacySafeUndispatched(r) && r.ResolveBy <= m.now().Unix())) {
				records = append(records, r)
				if len(records) >= limit {
					break
				}
			}
		}
		return nil
	}); e != nil {
		return 0, e
	}
	completed := 0
	for _, r := range records {
		var receipt LegacyCreditCall
		var e error
		if r.ResultDigest != "" {
			receipt, e = legacyFinishCore(ctx, port, r)
		} else {
			receipt, e = port.ReleaseLegacyCredits(ctx, LegacyCreditRelease{LegacyCreditID: LegacyCreditID{Owner: r.Owner, CallID: r.Core.ID}, Reason: "expired"})
		}
		if e != nil {
			return completed, e
		}
		if !legacyCallMatches(receipt, r) {
			return completed, ErrDenied
		}
		r.Core = receipt
		if receipt.State == "settled" || receipt.State == "released" {
			r.State = "done"
			completed++
		}
		if r.ResultExpires > 0 && r.ResultExpires <= m.now().Unix() {
			r.Result = nil
		}
		if e = m.legacySave(r); e != nil {
			return completed, e
		}
	}
	return completed, nil
}

// LegacyCancelExecution only releases an undispatched reservation. Uncertain
// remote work cannot be converted into a definitive failure by a client.
func (m *Module) LegacyCancelExecution(ctx context.Context, credential, account, id string) (LegacyExecution, error) {
	r, e := m.LegacyGetExecution(ctx, credential, account, id)
	if e != nil && e != ErrPending {
		return LegacyExecution{}, e
	}
	if r.Core.State == "released" {
		return r, nil
	}
	if !legacySafeUndispatched(r) {
		return r, ErrConflict
	}
	port := m.authority.(CoreLegacyCredits)
	receipt, e := port.ReleaseLegacyCredits(ctx, LegacyCreditRelease{LegacyCreditID: LegacyCreditID{Owner: r.Owner, CallID: r.Core.ID}, Reason: "cancelled"})
	if e != nil {
		return r, ErrPending
	}
	if !legacyCallMatches(receipt, r) || receipt.State != "released" {
		return r, ErrDenied
	}
	r.Core = receipt
	r.State = "done"
	if e = m.legacySave(r); e != nil {
		return r, ErrPending
	}
	return r, nil
}

type LegacyMeteringBinding struct {
	ServiceID, VersionID, Endpoint, ToolID, RemoteDigest string
}

// Implemented only by trusted host configuration; never populated by a request.
type LegacyMeteringResolver interface {
	ResolveLegacyMetering(context.Context, LegacyMeteringBinding) (*LegacyMeteringConfig, error)
}

// Delivery integrity and supplier authority use different frozen digests.
func legacyAuthorityDigest(r LegacyExecution) string {
	if r.AuthorityResultDigest != "" {
		return r.AuthorityResultDigest
	}
	return r.ResultDigest
}
func legacyFinishRequest(r LegacyExecution) LegacyCreditFinish {
	proof := LegacyProofOrdinary
	if r.Core.Held > 0 && len(r.RawResult) > 0 && r.AuthorityResultDigest != "" && r.Pricing.BillingMode == "" && r.ProviderPricing == nil && r.ProviderQuote == nil && r.Usage == nil {
		proof = LegacyProofFixedWitness
	}
	return LegacyCreditFinish{LegacyCreditID: LegacyCreditID{Owner: r.Owner, CallID: r.Core.ID, DispatchID: r.Core.DispatchID}, Success: r.Success, ResultDigest: legacyAuthorityDigest(r), Usage: nil, RawResult: base64.StdEncoding.EncodeToString(r.RawResult), ProofKind: proof}
}

func legacyFinishCore(ctx context.Context, port CoreLegacyCredits, r LegacyExecution) (LegacyCreditCall, error) {
	request := legacyFinishRequest(r)
	if request.ProofKind == LegacyProofFixedWitness {
		witness, ok := port.(LegacyFixedWitness)
		if !ok || missingInterface(witness) || !witness.FixedWitnessAvailable() {
			return LegacyCreditCall{}, ErrUnavailable
		}
	}
	return port.FinishLegacyCredits(ctx, request)
}
