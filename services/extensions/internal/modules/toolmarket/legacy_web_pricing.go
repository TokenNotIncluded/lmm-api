// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package toolmarket

import "encoding/json"

func (t WebTool) LegacyPricing() LegacyPricing {
	return LegacyPricing{PriceQuota: CreditAmount(t.PriceQuota), BillingMode: t.BillingMode, InputTokenPriceQuota: CreditAmount(t.InputTokenPriceQuota), MaxInputTokens: t.MaxInputTokens, BillingRules: append([]LegacyBillingRule(nil), t.BillingRules...)}
}
func normalizeWebToolPricing(endpoint string, t WebToolInput) (WebToolInput, *LegacyProviderPricing, error) {
	var provider *LegacyProviderPricing
	if len(t.ProviderPricing) > 0 && string(t.ProviderPricing) != "null" {
		p, err := payload[LegacyProviderPricing](t.ProviderPricing, "provider", "multiplier")
		if err != nil {
			return t, nil, ErrPrice
		}
		provider = &p
	}
	if err := ValidateLegacyProviderTool(endpoint, t.Name, provider); err != nil {
		return t, nil, err
	}
	var rules []LegacyBillingRule
	if len(t.BillingRules) > 0 && string(t.BillingRules) != "null" && decodeStrict(t.BillingRules, &rules) != nil {
		return t, nil, ErrInvalid
	}
	p, err := NormalizeLegacyPricing(LegacyPricing{PriceQuota: CreditAmount(t.PriceQuota), BillingMode: t.BillingMode, InputTokenPriceQuota: CreditAmount(t.InputTokenPriceQuota), MaxInputTokens: t.MaxInputTokens, BillingRules: rules})
	if err != nil {
		return t, nil, err
	}
	if provider != nil && p.BillingMode != "" {
		return t, nil, ErrPrice
	}
	t.PriceQuota = int64(p.PriceQuota)
	t.BillingMode = p.BillingMode
	t.InputTokenPriceQuota = int64(p.InputTokenPriceQuota)
	t.MaxInputTokens = p.MaxInputTokens
	if len(p.BillingRules) > 0 {
		t.BillingRules, _ = json.Marshal(p.BillingRules)
	} else {
		t.BillingRules = nil
	}
	return t, provider, nil
}
func normalizeWebDraftPricing(in WebDraftInput) (WebDraftInput, error) {
	in.Tools = append([]WebToolInput(nil), in.Tools...)
	for i, t := range in.Tools {
		normalized, _, err := normalizeWebToolPricing(in.Endpoint, t)
		if err != nil {
			return in, err
		}
		in.Tools[i] = normalized
	}
	return in, nil
}
func webToolPricingFields(t WebToolInput) (*LegacyProviderPricing, []LegacyBillingRule) {
	var provider *LegacyProviderPricing
	if len(t.ProviderPricing) > 0 && string(t.ProviderPricing) != "null" {
		var p LegacyProviderPricing
		if json.Unmarshal(t.ProviderPricing, &p) == nil {
			provider = &p
		}
	}
	var rules []LegacyBillingRule
	if len(t.BillingRules) > 0 {
		json.Unmarshal(t.BillingRules, &rules)
	}
	return provider, rules
}
