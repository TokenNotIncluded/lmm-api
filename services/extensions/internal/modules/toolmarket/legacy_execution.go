// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
// Execution ordering adapted from frozen main 3948a0a service/tool_market_remote.go.
package toolmarket

import (
	"context"
	"encoding/json"
	"errors"
	"strconv"
	"time"
)

// LegacyExecuteInput never accepts a payer, seller, fee or client asserted price.
// Core derives the personal wallet from the authenticated principal and freezes
// economics from its registered exact-version quote.
type LegacyExecuteInput struct {
	ClientID   string          `json:"client_id"`
	RequestKey string          `json:"request_key"`
	ToolID     string          `json:"tool_id"`
	VersionID  string          `json:"version_id"`
	GrantID    string          `json:"grant_id"`
	Arguments  json.RawMessage `json:"arguments"`
}

type LegacyExecution struct {
	SupplierReceiptAdmitted bool                    `json:"supplier_receipt_admitted"`
	Builtin                 *LegacyBuiltinExecution `json:"builtin_private,omitempty"`
	RawResult               []byte                  `json:"raw_result_private,omitempty"`
	AuthorityResultDigest   string                  `json:"authority_result_digest,omitempty"`
	ProviderPricing         *LegacyProviderPricing  `json:"provider_pricing,omitempty"`
	ProviderQuote           *LegacyProviderQuote    `json:"provider_quote,omitempty"`
	ProviderOwnerID         int64                   `json:"provider_owner_id,omitempty"`
	ProviderRunID           string                  `json:"provider_run_id,omitempty"`
	ProviderNextPollAt      int64                   `json:"provider_next_poll_at,omitempty"`
	ProviderPollLeaseUntil  int64                   `json:"provider_poll_lease_until,omitempty"`
	Pricing                 LegacyPricing           `json:"pricing"`
	SupplierPricingDigest   string                  `json:"supplier_pricing_digest,omitempty"`
	SettlementPricingDigest string                  `json:"settlement_pricing_digest"`
	MeteringBinding         LegacyMeteringBinding   `json:"metering_binding"`
	ID                      string                  `json:"id"`
	Owner                   Principal               `json:"owner"`
	Input                   LegacyExecuteInput      `json:"input"`
	RequestHash             string                  `json:"request_hash"`
	InputDigest             string                  `json:"input_digest"`
	QuoteID                 string                  `json:"quote_id"`
	ResolveBy               int64                   `json:"resolve_by"`
	State                   string                  `json:"state"`
	Core                    LegacyCreditCall        `json:"core"`
	Result                  json.RawMessage         `json:"result,omitempty"`
	Success                 bool                    `json:"success"`
	Usage                   *LegacyCreditUsage      `json:"usage,omitempty"`
	ResultDigest            string                  `json:"result_digest,omitempty"`
	ResultExpires           int64                   `json:"result_expires,omitempty"`
	CreatedAt               int64                   `json:"created_at"`
}

// LegacyExecuteRemote persists an operation before attempting Core reserve. A
// replay can reconcile that same reserve but can never execute tools/call twice.
func (m *Module) LegacyExecuteRemote(ctx context.Context, credential, account string, in LegacyExecuteInput) (LegacyExecution, error) {
	return m.legacyExecute(ctx, credential, account, in, false)
}

// LegacyExecute selects remote or an exact compiled builtin while sharing the
// same Core consent/reserve/dispatch and durable operation protocol.
func (m *Module) LegacyExecute(ctx context.Context, credential, account string, in LegacyExecuteInput) (LegacyExecution, error) {
	return m.legacyExecute(ctx, credential, account, in, true)
}
func (m *Module) legacyExecute(ctx context.Context, credential, account string, in LegacyExecuteInput, allowBuiltin bool) (LegacyExecution, error) {
	user, e := m.WebAuthorize(ctx, credential, account, "toolmarket.execute")
	if e != nil {
		return LegacyExecution{}, e
	}
	port, ok := m.authority.(CoreLegacyCredits)
	if !ok || missingInterface(port) {
		return LegacyExecution{}, ErrUnavailable
	}
	owner := Principal{User: strconv.FormatInt(user, 10), Account: account}
	if !legacyClientValid(in.ClientID) || len(in.RequestKey) == 0 || len(in.RequestKey) > 128 || !validID.MatchString(in.ToolID) || !validID.MatchString(in.VersionID) || !validID.MatchString(in.GrantID) {
		return LegacyExecution{}, ErrInvalid
	}
	var args any
	if e = decodeStrict(in.Arguments, &args); e != nil {
		return LegacyExecution{}, e
	}
	in.Arguments, _ = json.Marshal(args)
	id := digest([]any{user, in.ClientID, in.RequestKey})
	hash := digest(in)
	var prior LegacyExecution
	found := false
	e = m.store.view(func(d database) error {
		if r, ok := d.LegacyExecutions[id]; ok {
			if r.Owner != owner || r.RequestHash != hash {
				return ErrConflict
			}
			prior = r
			found = true
		}
		return nil
	})
	if e != nil {
		return LegacyExecution{}, e
	}
	if found {
		return m.legacyRecoverExecution(ctx, port, credential, prior)
	}
	select {
	case m.active <- struct{}{}:
		defer func() { <-m.active }()
	default:
		return LegacyExecution{}, ErrLimit
	}
	var snapshot webSnapshot
	var tool WebTool
	var remoteCredential webCredential
	var grant LegacyConsentGrant
	var builtin *LegacyBuiltinExecution
	e = m.store.view(func(d database) error {
		var err error
		tool, err = legacyLiveTool(&d, user, in.ToolID, in.VersionID)
		if err != nil {
			return err
		}
		snapshot = d.Web.Versions[in.VersionID]
		if (snapshot.Version.ExecutionType != "remote" && snapshot.Version.ExecutionType != "builtin") || len(snapshot.RemoteDigests[in.ToolID]) != 64 {
			return ErrConflict
		}
		installation, ok := d.Web.Installations[digest([]any{user, in.ClientID, in.ToolID})]
		if !ok || installation.VersionID != in.VersionID {
			return ErrDenied
		}
		grant, ok = legacyAccess(&d).Grants[in.GrantID]
		if !ok || grant.UserID != user || grant.ClientID != in.ClientID || grant.ToolID != in.ToolID || grant.VersionID != in.VersionID || grant.RevokedAt != 0 || grant.ExpiresAt <= m.now().Unix() {
			return ErrConsent
		}
		if snapshot.Version.ExecutionType == "builtin" {
			if !allowBuiltin || missingInterface(m.builtinRuntime) {
				return ErrUnavailable
			}
			builtin, err = legacyCompiledExecution(snapshot, tool)
			return err
		}
		service := d.Web.Services[snapshot.Version.ServiceID]
		remoteCredential, err = webResolveCredential(d, service.OwnerID, service.ID, in.VersionID, snapshot.Version.Endpoint)
		return err
	})
	if e != nil {
		return LegacyExecution{}, e
	}
	if _, e = ValidateLegacyArguments(json.RawMessage(tool.InputSchema), in.Arguments); e != nil {
		return LegacyExecution{}, e
	}
	normalizedPricing, err := NormalizeLegacyPricing(tool.LegacyPricing())
	if err != nil {
		return LegacyExecution{}, err
	}
	// Provider pricing requires a separate Core financial contract.
	if tool.ProviderPricing != nil {
		return LegacyExecution{}, ErrUnavailable
	}
	paidMetered := normalizedPricing.BillingMode != "" && normalizedPricing.PriceQuota > 0
	if normalizedPricing.BillingMode == "" && normalizedPricing.PriceQuota > 0 {
		witness, ok := m.authority.(LegacyFixedWitness)
		if !ok || missingInterface(witness) || !witness.FixedWitnessAvailable() {
			return LegacyExecution{}, ErrUnavailable
		}
		if err := witness.AdmitFixedWitness(ctx, grant.QuoteID); err != nil {
			return LegacyExecution{}, err
		}
	}
	callctx, cancel := context.WithTimeout(ctx, 45*time.Second)
	defer cancel()
	var connection *mcpSession
	discover := func() error {
		connection, e = m.legacyConnect(callctx, snapshot.Version.Endpoint, remoteCredential)
		if e != nil {
			return e
		}
		remote, e := connection.legacyTools(callctx)
		if e != nil {
			return e
		}
		matched := false
		for _, t := range remote {
			if t.Name == tool.Name && digest(t) == snapshot.RemoteDigests[in.ToolID] {
				matched = true
				break
			}
		}
		if !matched {
			return ErrConflict
		}
		if tool.ProviderPricing != nil {
			return ErrUnavailable
		}
		if e = ValidateLegacyProviderTool(snapshot.Version.Endpoint, tool.Name, nil); e != nil {
			return e
		}
		for _, preset := range LegacyProviderPresets() {
			if snapshot.Version.Endpoint == preset.Endpoint && (tool.Name == preset.InspectTool || tool.Name == preset.ExecuteTool) {
				values, ok := args.(map[string]any)
				if !ok {
					return ErrInvalid
				}
				if _, err := LegacyProviderQuoteArguments(preset.ID, values); err != nil {
					return err
				}
			}
		}

		return nil
	}
	defer func() {
		if connection != nil {
			connection.close(callctx)
		}
	}()
	if builtin == nil && !paidMetered {
		if err := discover(); err != nil {
			return LegacyExecution{}, err
		}
	}
	resolveBy := m.now().Unix() + 120
	if grant.ExpiresAt < resolveBy {
		resolveBy = grant.ExpiresAt
	}
	settlementPricingDigest, err := LegacyQuotePricingDigest(tool.LegacyPricing())
	if err != nil {
		return LegacyExecution{}, err
	}
	supplierPricingDigest := ""
	if tool.BillingMode != "" {
		supplierPricingDigest, err = LegacyMeteringPricingDigest(tool.LegacyPricing())
		if err != nil {
			return LegacyExecution{}, err
		}
	}
	record := LegacyExecution{Builtin: builtin, Pricing: tool.LegacyPricing(), SupplierPricingDigest: supplierPricingDigest, SettlementPricingDigest: settlementPricingDigest, MeteringBinding: LegacyMeteringBinding{ServiceID: snapshot.Version.ServiceID, VersionID: in.VersionID, Endpoint: snapshot.Version.Endpoint, ToolID: in.ToolID, RemoteDigest: snapshot.RemoteDigests[in.ToolID]}, ID: id, Owner: owner, Input: in, RequestHash: hash, InputDigest: digest(args), QuoteID: grant.QuoteID, State: "prepared", ResolveBy: resolveBy, CreatedAt: m.now().Unix()}
	created := false
	e = m.store.update(func(d *database) error {
		if d.LegacyExecutions == nil {
			d.LegacyExecutions = map[string]LegacyExecution{}
		}
		if r, ok := d.LegacyExecutions[id]; ok {
			if r.Owner != owner || r.RequestHash != hash {
				return ErrConflict
			}
			record = r
			return nil
		}
		if len(d.LegacyExecutions) >= 50000 {
			return ErrLimit
		}
		// Recheck the catalog after discovery, immediately before recording intent.
		if _, err := legacyLiveTool(d, user, in.ToolID, in.VersionID); err != nil {
			return err
		}
		d.LegacyExecutions[id] = record
		created = true
		return nil
	})
	if e != nil {
		return LegacyExecution{}, e
	}
	if !created {
		return m.legacyRecoverExecution(ctx, port, credential, record)
	}
	receipt, e := port.ReserveLegacyCredits(ctx, credential, LegacyCreditReserve{Owner: owner, ClientID: in.ClientID, RequestKey: in.RequestKey, QuoteID: record.QuoteID, GrantID: in.GrantID, ToolID: in.ToolID, VersionID: in.VersionID, InputDigest: record.InputDigest, ResolveBy: record.ResolveBy})
	if e != nil {
		return record, ErrPending
	}
	if !legacyCallMatches(receipt, record) {
		return record, ErrDenied
	}
	record.Core = receipt
	if receipt.State != "reserved" || receipt.DispatchID != "" {
		record.State = "unknown"
		if m.legacySave(record) != nil {
			return record, ErrPending
		}
		return m.legacyRecoverExecution(ctx, port, credential, record)
	}
	record.SupplierReceiptAdmitted = paidMetered
	record.State = "reserved"
	if e = m.legacySave(record); e != nil {
		return record, ErrPending
	}
	// Core commits the immutable supplier receipt contract and verifier admission
	// before even MCP discovery is permitted for a paid metered execution.
	if builtin == nil && paidMetered {
		if err := discover(); err != nil {
			return record, err
		}
	}
	if _, e = m.WebAuthorize(ctx, credential, account, "toolmarket.execute"); e != nil {
		return record, ErrPending
	}
	// Persist the no-retry fence before the authority dispatch claim. A lost claim
	// response can therefore only cause unknown/recovery, never another execution.
	record.State = "unknown"
	if e = m.store.update(func(d *database) error {
		if _, err := legacyLiveTool(d, user, in.ToolID, in.VersionID); err != nil {
			return err
		}
		installation, ok := d.Web.Installations[digest([]any{user, in.ClientID, in.ToolID})]
		current := d.Web.Versions[in.VersionID]
		if !ok || installation.VersionID != in.VersionID || current.Digest != snapshot.Digest || current.RemoteDigests[in.ToolID] != snapshot.RemoteDigests[in.ToolID] {
			return ErrConflict
		}
		g, ok := legacyAccess(d).Grants[in.GrantID]
		if !ok || g.RevokedAt != 0 || g.ExpiresAt <= m.now().Unix() {
			return ErrConsent
		}
		old, ok := d.LegacyExecutions[id]
		if !ok || old.State != "reserved" || old.RequestHash != hash {
			return ErrConflict
		}
		d.LegacyExecutions[id] = record
		return nil
	}); e != nil {
		return record, ErrPending
	}
	dispatched, e := port.DispatchLegacyCredits(ctx, credential, LegacyCreditID{Owner: owner, CallID: receipt.ID})
	if e != nil {
		return record, ErrPending
	}
	if !legacyCallMatches(dispatched, record) || !dispatched.DispatchClaimed || dispatched.State != "running" {
		return record, ErrPending
	}
	record.Core = dispatched
	record.Core.DispatchClaimed = false
	if e = m.legacySave(record); e != nil {
		return record, ErrPending
	}
	if record.Builtin != nil {
		result, err := m.builtinRuntime.InvokeBuiltin(ctx, credential, legacyBuiltinInvocation(record, builtinPeerIP(ctx)))
		if err != nil {
			record.Builtin.Indeterminate = true
			_ = m.legacySave(record)
			_, _ = port.UnknownLegacyCredits(ctx, LegacyCreditID{Owner: owner, CallID: record.Core.ID, DispatchID: record.Core.DispatchID})
			return record, ErrPending
		}
		return m.legacySaveBuiltinOutcome(ctx, port, credential, record, tool, result)
	}
	params := map[string]any{"name": tool.Name, "arguments": args}
	if tool.LegacyPricing().BillingMode != "" {
		pricingDigest, err := LegacyMeteringPricingDigest(tool.LegacyPricing())
		if err != nil {
			return record, ErrPending
		}
		params["_meta"] = map[string]any{"lmm_metering": LegacyMeteringContext{CallID: record.Core.ID, VersionID: in.VersionID, ToolID: in.ToolID, InputDigest: record.InputDigest, PricingDigest: pricingDigest}}
	}
	result, e := connection.rpc(callctx, "tools/call", params)
	if errors.Is(e, ErrAuth) {
		return m.legacyFinishRemote(ctx, port, record, tool, legacyFailureResult("The remote service rejected its configured credential."), credential, remoteCredential.Secret)
	}
	if e != nil {
		_, _ = port.UnknownLegacyCredits(ctx, LegacyCreditID{Owner: owner, CallID: dispatched.ID, DispatchID: dispatched.DispatchID})
		return record, ErrPending
	}
	return m.legacyFinishRemote(ctx, port, record, tool, result, credential, remoteCredential.Secret)
}

func (m *Module) legacyConnect(ctx context.Context, endpoint string, c webCredential) (*mcpSession, error) {
	s := Server{Endpoint: endpoint, Auth: "none"}
	secret := Secret{}
	if c.Mode != "" && c.Mode != "none" {
		if !validWebCredential(c.Mode, c.Secret) {
			return nil, ErrInvalid
		}
		s.Auth = "api_key"
		s.Header = "Authorization"
		if c.Mode == "api_key" {
			s.Header = "X-Api-Key"
		}
		secret.Access = c.Secret
	}
	return m.connectLegacy(ctx, s, secret)
}
func (m *Module) legacySave(r LegacyExecution) error {
	r.Core.DispatchClaimed = false
	return m.store.update(func(d *database) error {
		old, ok := d.LegacyExecutions[r.ID]
		if !ok || old.Owner != r.Owner || old.RequestHash != r.RequestHash {
			return ErrConflict
		}
		if old.ResultDigest != "" && old.ResultDigest != r.ResultDigest {
			return ErrConflict
		}
		r.SupplierReceiptAdmitted = r.SupplierReceiptAdmitted || old.SupplierReceiptAdmitted
		if old.Builtin != nil && r.Builtin != nil {
			// A stale query cannot undo the durable one-use resume fence or
			// turn an ambiguous downstream execution back into a resumable preview.
			r.Builtin.ResumeClaimed = r.Builtin.ResumeClaimed || old.Builtin.ResumeClaimed
			r.Builtin.Indeterminate = r.Builtin.Indeterminate || old.Builtin.Indeterminate
			if r.ResultDigest == "" && (r.Builtin.ResumeClaimed || r.Builtin.Indeterminate) {
				r.State = "unknown"
			}
		}
		if old.State == "done" {
			r.Core = old.Core
			r.State = "done"
		}
		d.LegacyExecutions[r.ID] = r
		return nil
	})
}
