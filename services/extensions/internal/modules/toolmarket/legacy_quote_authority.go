// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package toolmarket

import (
	"context"
	"encoding/json"
	"math"
	"strconv"
)

// LegacyQuotePrice is the Core CREDIT-v2 tagged price, not the historical
// supplier HMAC rules-only digest and not native USD micro-units.
type LegacyQuotePrice struct {
	Mode       string
	PriceQuota CreditAmount
	Rules      []LegacyBillingRule
}

func LegacyQuotePriceFromPricing(p LegacyPricing) (LegacyQuotePrice, error) {
	normalized, err := NormalizeLegacyPricing(p)
	if err != nil {
		return LegacyQuotePrice{}, err
	}
	if normalized.BillingMode == "" {
		return LegacyQuotePrice{Mode: "fixed", PriceQuota: normalized.PriceQuota}, nil
	}
	return LegacyQuotePrice{Mode: "metered", Rules: append([]LegacyBillingRule(nil), legacyPricingRules(normalized)...)}, nil
}
func (p LegacyQuotePrice) MarshalJSON() ([]byte, error) {
	switch p.Mode {
	case "fixed":
		if !p.PriceQuota.Valid() || len(p.Rules) != 0 {
			return nil, ErrInvalid
		}
		return json.Marshal(struct {
			Mode       string       `json:"mode"`
			PriceQuota CreditAmount `json:"price_quota"`
		}{p.Mode, p.PriceQuota})
	case "metered":
		if p.PriceQuota != 0 {
			return nil, ErrInvalid
		}
		if _, err := NormalizeLegacyPricing(LegacyPricing{BillingMode: "metered", BillingRules: p.Rules}); err != nil {
			return nil, err
		}
		return json.Marshal(struct {
			Mode  string              `json:"mode"`
			Rules []LegacyBillingRule `json:"rules"`
		}{p.Mode, p.Rules})
	default:
		return nil, ErrInvalid
	}
}
func (p *LegacyQuotePrice) UnmarshalJSON(raw []byte) error {
	var kind struct {
		Mode       string          `json:"mode"`
		PriceQuota json.RawMessage `json:"price_quota,omitempty"`
		Rules      json.RawMessage `json:"rules,omitempty"`
	}
	if err := decodeStrict(raw, &kind); err != nil {
		return err
	}
	switch kind.Mode {
	case "fixed":
		var fixed struct {
			Mode       string       `json:"mode"`
			PriceQuota CreditAmount `json:"price_quota"`
		}
		if err := decodeStrict(raw, &fixed); err != nil {
			return err
		}
		if err := legacyRequiredFields(raw, "mode", "price_quota"); err != nil {
			return err
		}
		*p = LegacyQuotePrice{Mode: "fixed", PriceQuota: fixed.PriceQuota}
	case "metered":
		var metered struct {
			Mode  string              `json:"mode"`
			Rules []LegacyBillingRule `json:"rules"`
		}
		if err := decodeStrict(raw, &metered); err != nil {
			return err
		}
		if err := legacyRequiredFields(raw, "mode", "rules"); err != nil {
			return err
		}
		var rawRules []json.RawMessage
		if err := json.Unmarshal(kind.Rules, &rawRules); err != nil {
			return ErrInvalid
		}
		for _, rule := range rawRules {
			if err := legacyRequiredFields(rule, "metric", "rate_quota", "max_quantity"); err != nil {
				return err
			}
		}
		if _, err := NormalizeLegacyPricing(LegacyPricing{BillingMode: "metered", BillingRules: metered.Rules}); err != nil {
			return err
		}
		*p = LegacyQuotePrice{Mode: "metered", Rules: metered.Rules}
	default:
		return ErrInvalid
	}
	return nil
}
func LegacyQuotePricingDigest(p LegacyPricing) (string, error) {
	price, err := LegacyQuotePriceFromPricing(p)
	if err != nil {
		return "", err
	}
	return digest(price), nil
}

type LegacyCreditQuote struct {
	ID            string           `json:"id"`
	ServiceID     string           `json:"service_id"`
	ToolID        string           `json:"tool_id"`
	VersionID     string           `json:"version_id"`
	RemoteDigest  string           `json:"remote_digest"`
	PricingDigest string           `json:"pricing_digest"`
	Price         LegacyQuotePrice `json:"price"`
	Expires       int64            `json:"expires"`
}
type LegacyQuoteApproval struct {
	OperationID string `json:"operation_id"`
	QuoteID     string `json:"quote_id"`
	Visibility  string `json:"visibility"`
	Approved    bool   `json:"approved"`
}
type LegacyQuoteApprovalReceipt struct {
	LegacyQuoteApproval
	ApprovedBy string `json:"approved_by"`
	ApprovedAt int64  `json:"approved_at"`
}
type CoreLegacyQuotes interface {
	RegisterLegacyQuote(context.Context, string, LegacyCreditQuote) (LegacyCreditQuote, error)
	ApproveLegacyQuote(context.Context, string, LegacyQuoteApproval) (LegacyQuoteApprovalReceipt, error)
}

func (q *LegacyCreditQuote) UnmarshalJSON(raw []byte) error {
	type alias LegacyCreditQuote
	var next alias
	if err := decodeStrict(raw, &next); err != nil {
		return err
	}
	if err := legacyRequiredFields(raw, "id", "service_id", "tool_id", "version_id", "remote_digest", "pricing_digest", "price", "expires"); err != nil {
		return err
	}
	*q = LegacyCreditQuote(next)
	return nil
}
func (r *LegacyQuoteApprovalReceipt) UnmarshalJSON(raw []byte) error {
	type alias LegacyQuoteApprovalReceipt
	var next alias
	if err := decodeStrict(raw, &next); err != nil {
		return err
	}
	if err := legacyRequiredFields(raw, "operation_id", "quote_id", "visibility", "approved", "approved_by", "approved_at"); err != nil {
		return err
	}
	*r = LegacyQuoteApprovalReceipt(next)
	return nil
}
func legacyVersionCreditQuotes(serviceID string, v webSnapshot) ([]LegacyCreditQuote, error) {
	if !validID.MatchString(serviceID) || v.Version.ServiceID != serviceID || !validID.MatchString(v.Version.ID) || v.Version.ExecutionType != "remote" || !legacyRulesHash(v.Digest) || v.Version.Digest != v.Digest || v.ValidationDigest != v.Digest || len(v.Tools) < 1 || len(v.Tools) > 100 {
		return nil, ErrConflict
	}
	seen := map[string]bool{}
	out := make([]LegacyCreditQuote, 0, len(v.Tools))
	for _, tool := range v.Tools {
		if !validID.MatchString(tool.ToolID) || seen[tool.ToolID] || tool.VersionID != v.Version.ID || !legacyRulesHash(v.RemoteDigests[tool.ToolID]) {
			return nil, ErrConflict
		}
		if tool.ProviderPricing != nil {
			return nil, ErrPrice
		}
		seen[tool.ToolID] = true
		price, err := LegacyQuotePriceFromPricing(tool.LegacyPricing())
		if err != nil {
			return nil, err
		}
		pricingDigest, err := LegacyQuotePricingDigest(tool.LegacyPricing())
		if err != nil {
			return nil, err
		}
		// Static reviewed publications have no quote clock refresh or new ID on retry.
		id := digest([]string{"legacy-credit-quote-v2", v.Version.ID, tool.ToolID, v.RemoteDigests[tool.ToolID], pricingDigest})
		out = append(out, LegacyCreditQuote{ID: id, ServiceID: serviceID, ToolID: tool.ToolID, VersionID: v.Version.ID, RemoteDigest: v.RemoteDigests[tool.ToolID], PricingDigest: pricingDigest, Price: price, Expires: math.MaxInt64})
	}
	return out, nil
}
func (m *Module) legacyQuoteSnapshot(serviceID, versionID string) (WebServiceRecord, webSnapshot, error) {
	var service WebServiceRecord
	var snapshot webSnapshot
	err := m.store.view(func(d database) error {
		if d.Web == nil {
			return ErrMissing
		}
		var ok bool
		service, ok = d.Web.Services[serviceID]
		if !ok || service.Status == "deleted" {
			return ErrMissing
		}
		snapshot, ok = d.Web.Versions[versionID]
		if !ok || snapshot.Version.ServiceID != serviceID {
			return ErrMissing
		}
		if service.DraftVersionID != versionID && service.LiveVersionID != versionID {
			return ErrConflict
		}
		return nil
	})
	return service, snapshot, err
}
func (m *Module) legacyQuotesUnchanged(service WebServiceRecord, snapshot webSnapshot) error {
	current, v, err := m.legacyQuoteSnapshot(service.ID, snapshot.Version.ID)
	if err != nil {
		return err
	}
	if current.OwnerID != service.OwnerID || current.DraftVersionID != service.DraftVersionID || current.LiveVersionID != service.LiveVersionID || current.Status != service.Status || digest(v) != digest(snapshot) {
		return ErrConflict
	}
	return nil
}
func (m *Module) legacyRegisterVersionQuotes(ctx context.Context, credential, account, serviceID, versionID string) error {
	actor, err := m.WebAuthorize(ctx, credential, account, "toolmarket.manage")
	if err != nil {
		return err
	}
	port, ok := m.authority.(CoreLegacyQuotes)
	if !ok || missingInterface(port) {
		return ErrUnavailable
	}
	service, v, err := m.legacyQuoteSnapshot(serviceID, versionID)
	if err != nil {
		return err
	}
	if service.OwnerID != actor {
		return ErrDenied
	}
	quotes, err := legacyVersionCreditQuotes(serviceID, v)
	if err != nil {
		return err
	}
	for _, q := range quotes {
		if err := m.legacyQuotesUnchanged(service, v); err != nil {
			return err
		}
		receipt, err := port.RegisterLegacyQuote(ctx, credential, q)
		if err != nil {
			return err
		}
		if digest(receipt) != digest(q) {
			return ErrDenied
		}
	}
	if current, err := m.WebAuthorize(ctx, credential, account, "toolmarket.manage"); err != nil {
		return err
	} else if current != actor {
		return ErrDenied
	}
	return m.legacyQuotesUnchanged(service, v)
}
func (m *Module) legacyApproveVersionQuotes(ctx context.Context, credential, account, serviceID, versionID, visibility string, approved bool) error {
	actor, err := m.WebAuthorize(ctx, credential, account, "toolmarket.read")
	if err != nil {
		return err
	}
	port, ok := m.authority.(CoreLegacyQuotes)
	if !ok || missingInterface(port) {
		return ErrUnavailable
	}
	service, v, err := m.legacyQuoteSnapshot(serviceID, versionID)
	if err != nil {
		return err
	}
	quotes, err := legacyVersionCreditQuotes(serviceID, v)
	if err != nil {
		return err
	}
	expectedVisibility := "public"
	if v.Version.Visibility == "private" {
		expectedVisibility = "private"
	} else if v.Version.Visibility != "public" && v.Version.Visibility != "shared" {
		return ErrInvalid
	}
	if visibility != expectedVisibility {
		return ErrDenied
	}
	permission := "toolmarket.review"
	if actor == service.OwnerID && (!approved || visibility == "private") {
		permission = "toolmarket.manage"
	}
	if approved && visibility == "private" {
		if actor != service.OwnerID {
			return ErrDenied
		}
		for _, q := range quotes {
			ceiling, err := q.Price.Ceiling()
			if err != nil || ceiling != 0 {
				return ErrDenied
			}
		}
	}
	if current, err := m.WebAuthorize(ctx, credential, account, permission); err != nil {
		return err
	} else if current != actor {
		return ErrDenied
	}
	for _, q := range quotes {
		if err := m.legacyQuotesUnchanged(service, v); err != nil {
			return err
		}
		operation := digest([]any{"legacy-quote-approve-v2", actor, q.ID, visibility, approved})
		req := LegacyQuoteApproval{OperationID: operation, QuoteID: q.ID, Visibility: visibility, Approved: approved}
		receipt, err := port.ApproveLegacyQuote(ctx, credential, req)
		if err != nil {
			return err
		}
		if receipt.LegacyQuoteApproval != req || receipt.ApprovedBy != strconv.FormatInt(actor, 10) || receipt.ApprovedAt <= 0 || receipt.ApprovedAt > m.now().Unix() {
			return ErrDenied
		}
	}
	if current, err := m.WebAuthorize(ctx, credential, account, permission); err != nil {
		return err
	} else if current != actor {
		return ErrDenied
	}
	return m.legacyQuotesUnchanged(service, v)
}

func (p LegacyQuotePrice) Ceiling() (CreditAmount, error) {
	if p.Mode == "fixed" {
		if !p.PriceQuota.Valid() || len(p.Rules) != 0 {
			return 0, ErrInvalid
		}
		return p.PriceQuota, nil
	}
	if p.Mode != "metered" || p.PriceQuota != 0 {
		return 0, ErrInvalid
	}
	normalized, err := NormalizeLegacyPricing(LegacyPricing{BillingMode: "metered", BillingRules: p.Rules})
	if err != nil {
		return 0, err
	}
	return normalized.PriceQuota, nil
}
