// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
// Rules ported from fixed main 3948a0a apps/api-go/model/tool_market_{usage,
// metering,reported_usage,provider}.go and internal/marketprovider/pricing.go.
package toolmarket

import (
	"bytes"
	"crypto/hmac"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"io"
	"math/big"
	"reflect"
	"regexp"
)

// CreditAmount is the legacy wallet integer, never USD millionths. Core alone
// authorizes and mutates balances; these functions only validate pricing evidence.
type CreditAmount int64

const MaxCreditAmount CreditAmount = 9007199254740991
const LegacyCreditsPerUSD int64 = 500000

func (n CreditAmount) Valid() bool { return n >= 0 && n <= MaxCreditAmount }
func (n CreditAmount) MarshalJSON() ([]byte, error) {
	if !n.Valid() {
		return nil, ErrInvalid
	}
	return json.Marshal(int64(n))
}
func (n *CreditAmount) UnmarshalJSON(raw []byte) error {
	var v int64
	if json.Unmarshal(raw, &v) != nil || bytes.Equal(bytes.TrimSpace(raw), []byte("null")) || !CreditAmount(v).Valid() {
		return ErrInvalid
	}
	*n = CreditAmount(v)
	return nil
}

type LegacyBillingRule struct {
	Metric      string       `json:"metric"`
	RateQuota   CreditAmount `json:"rate_quota"`
	MaxQuantity int64        `json:"max_quantity"`
}
type LegacyPricing struct {
	PriceQuota           CreditAmount        `json:"price_quota"`
	BillingMode          string              `json:"billing_mode,omitempty"`
	InputTokenPriceQuota CreditAmount        `json:"input_token_price_quota,omitempty"`
	MaxInputTokens       int64               `json:"max_input_tokens,omitempty"`
	BillingRules         []LegacyBillingRule `json:"billing_rules,omitempty"`
}

var legacyMetricScales = map[string]int64{
	"input_tokens": 1000000, "output_tokens": 1000000,
	"input_characters": 1000, "output_characters": 1000, "images": 1,
	"audio_milliseconds": 1000, "video_milliseconds": 1000,
	"cpu_core_milliseconds": 1000, "memory_mib_seconds": 1024,
	"gpu_milliseconds": 1000, "vm_milliseconds": 1000, "storage_mib_seconds": 3686400,
}

// NormalizeLegacyPricing freezes the maximum hold. Actual metered settlement
// uses LegacyUsageCharge and must still be checked by Core against that hold.
func NormalizeLegacyPricing(p LegacyPricing) (LegacyPricing, error) {
	if !p.PriceQuota.Valid() {
		return LegacyPricing{}, ErrInvalid
	}
	p.BillingRules = append([]LegacyBillingRule(nil), p.BillingRules...)
	switch p.BillingMode {
	case "", "fixed":
		if len(p.BillingRules) != 0 || p.InputTokenPriceQuota != 0 || p.MaxInputTokens != 0 {
			return LegacyPricing{}, ErrInvalid
		}
		p.BillingMode = ""
	case "input_tokens":
		if len(p.BillingRules) != 0 || p.MaxInputTokens <= 0 || p.MaxInputTokens > 1000000 {
			return LegacyPricing{}, ErrInvalid
		}
		amount, err := legacyRulesCharge([]LegacyBillingRule{{"input_tokens", p.InputTokenPriceQuota, p.MaxInputTokens}}, nil)
		if err != nil {
			return LegacyPricing{}, err
		}
		p.PriceQuota = amount
	case "metered":
		if p.InputTokenPriceQuota != 0 || p.MaxInputTokens != 0 {
			return LegacyPricing{}, ErrInvalid
		}
		amount, err := legacyRulesCharge(p.BillingRules, nil)
		if err != nil {
			return LegacyPricing{}, err
		}
		p.PriceQuota = amount
	default:
		return LegacyPricing{}, ErrInvalid
	}
	return p, nil
}
func legacyPricingRules(p LegacyPricing) []LegacyBillingRule {
	if p.BillingMode == "input_tokens" {
		return []LegacyBillingRule{{"input_tokens", p.InputTokenPriceQuota, p.MaxInputTokens}}
	}
	return p.BillingRules
}
func LegacyUsageCharge(p LegacyPricing, quantities map[string]int64) (CreditAmount, error) {
	p, err := NormalizeLegacyPricing(p)
	if err != nil {
		return 0, err
	}
	if p.BillingMode == "" {
		if len(quantities) != 0 {
			return 0, ErrInvalid
		}
		return p.PriceQuota, nil
	}
	if quantities == nil {
		return 0, ErrInvalid
	}
	return legacyRulesCharge(legacyPricingRules(p), quantities)
}
func legacyRulesCharge(rules []LegacyBillingRule, quantities map[string]int64) (CreditAmount, error) {
	if len(rules) == 0 || len(rules) > len(legacyMetricScales) || (quantities != nil && len(quantities) != len(rules)) {
		return 0, ErrInvalid
	}
	total := new(big.Int)
	seen := map[string]bool{}
	for _, r := range rules {
		scale, ok := legacyMetricScales[r.Metric]
		if !ok || seen[r.Metric] || !r.RateQuota.Valid() || r.RateQuota == 0 || r.MaxQuantity <= 0 || r.MaxQuantity > 1000000000000 {
			return 0, ErrInvalid
		}
		seen[r.Metric] = true
		quantity := r.MaxQuantity
		if quantities != nil {
			var exists bool
			quantity, exists = quantities[r.Metric]
			if !exists || quantity < 0 || quantity > r.MaxQuantity {
				return 0, ErrInvalid
			}
		}
		part := new(big.Int).Mul(big.NewInt(int64(r.RateQuota)), big.NewInt(quantity))
		part.Add(part, big.NewInt(scale-1)).Quo(part, big.NewInt(scale))
		total.Add(total, part)
	}
	if !total.IsInt64() || !CreditAmount(total.Int64()).Valid() {
		return 0, ErrInvalid
	}
	return CreditAmount(total.Int64()), nil
}

// LegacyCreditSplit uses the historical floor fee, including when both payees
// happen to be the same account. It performs no ledger operation.
func LegacyCreditSplit(charged CreditAmount, bps int64) (seller, platform CreditAmount, err error) {
	if !charged.Valid() || bps < 0 || bps > 10000 {
		return 0, 0, ErrInvalid
	}
	fee := (int64(charged)/10000)*bps + ((int64(charged)%10000)*bps)/10000
	return charged - CreditAmount(fee), CreditAmount(fee), nil
}

type LegacyProviderQuote struct {
	TargetProvider string `json:"target_provider,omitempty"`
	TargetEndpoint string `json:"target_endpoint,omitempty"`
	Provider       string `json:"provider"`
	Unit           string `json:"unit"`
	AmountUSD      string `json:"amount_usd"`
	FlatFeeUSD     string `json:"flat_fee_usd,omitempty"`
}

var legacyDecimalPattern = regexp.MustCompile(`^(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]{1,2})?$`)

// fixed main pricing.go:decimal caps each USD component at 1m. Its
// parseDecimal is used only for the credit conversion basis, not these prices.
func legacyProviderDecimal(s string) (*big.Rat, error) {
	if len(s) == 0 || len(s) > 64 || !legacyDecimalPattern.MatchString(s) {
		return nil, ErrInvalid
	}
	n, ok := new(big.Rat).SetString(s)
	if !ok || n.Sign() < 0 || n.Cmp(big.NewRat(1000000, 1)) > 0 {
		return nil, ErrInvalid
	}
	return n, nil
}
func legacyQuoteCharge(q LegacyProviderQuote, multiplier string) (CreditAmount, error) {
	if (q.Provider != "agentkey" && q.Provider != "monid") || q.Unit != "call" {
		return 0, ErrInvalid
	}
	cost, err := legacyProviderDecimal(q.AmountUSD)
	if err != nil {
		return 0, err
	}
	if q.FlatFeeUSD != "" {
		flat, err := legacyProviderDecimal(q.FlatFeeUSD)
		if err != nil {
			return 0, err
		}
		cost.Add(cost, flat)
	}
	m, err := legacyProviderDecimal(multiplier)
	if err != nil || m.Cmp(big.NewRat(1, 1)) < 0 || m.Cmp(big.NewRat(100, 1)) > 0 {
		return 0, ErrInvalid
	}
	cost.Mul(cost, m).Mul(cost, big.NewRat(LegacyCreditsPerUSD, 1))
	n := new(big.Int).Add(cost.Num(), new(big.Int).Sub(cost.Denom(), big.NewInt(1)))
	n.Quo(n, cost.Denom())
	if !n.IsInt64() || !CreditAmount(n.Int64()).Valid() {
		return 0, ErrInvalid
	}
	return CreditAmount(n.Int64()), nil
}
func LegacyProviderCharge(q LegacyProviderQuote, multiplier string, ceiling CreditAmount, feeBPS int64) (CreditAmount, error) {
	if !ceiling.Valid() {
		return 0, ErrInvalid
	}
	price, err := legacyQuoteCharge(q, multiplier)
	if err != nil {
		return 0, err
	}
	cost, err := legacyQuoteCharge(q, "1")
	if err != nil {
		return 0, err
	}
	seller, _, err := LegacyCreditSplit(price, feeBPS)
	if err != nil || price > ceiling || seller < cost {
		return 0, ErrPrice
	}
	return price, nil
}

type LegacyMeteringContext struct {
	CallID        string `json:"call_id"`
	VersionID     string `json:"version_id"`
	ToolID        string `json:"tool_id"`
	InputDigest   string `json:"input_digest"`
	PricingDigest string `json:"pricing_digest"`
}

// Config must be resolved by trusted configuration pinned to the immutable
// service/version/endpoint/tool/remote digest. It must never come from a request.
type LegacyMeteringConfig struct {
	AdapterID string
	Context   LegacyMeteringContext
	Key       []byte
	Metrics   []string
}

func LegacyMeteringPricingDigest(p LegacyPricing) (string, error) {
	p, err := NormalizeLegacyPricing(p)
	if err != nil || p.BillingMode == "" {
		return "", ErrInvalid
	}
	return digest(legacyPricingRules(p)), nil
}

type LegacyUsageEvidence struct {
	Quantities   map[string]int64
	Source       string
	Report       json.RawMessage
	ResultDigest string
}
type legacyMeteringReceipt struct {
	AdapterID    string                `json:"adapter_id"`
	Context      LegacyMeteringContext `json:"context"`
	Quantities   map[string]int64      `json:"quantities"`
	Signature    string                `json:"signature"`
	ResultDigest string                `json:"result_digest"`
}

// ReadLegacyUsage preserves the two trust levels. A supplied signed receipt
// cannot fall back to unsigned usage on any verification failure.
func ReadLegacyUsage(p LegacyPricing, data json.RawMessage, config *LegacyMeteringConfig) (LegacyUsageEvidence, error) {
	p, err := NormalizeLegacyPricing(p)
	if err != nil || p.BillingMode == "" || len(data) > 2<<20 {
		return LegacyUsageEvidence{}, ErrInvalid
	}
	var result map[string]json.RawMessage
	if strictLegacyJSON(data, &result, false) != nil || result == nil {
		return LegacyUsageEvidence{}, ErrInvalid
	}
	var meta map[string]json.RawMessage
	if raw, ok := result["_meta"]; ok && strictLegacyJSON(raw, &meta, false) != nil {
		return LegacyUsageEvidence{}, ErrInvalid
	}
	if raw, ok := meta["lmm_metering"]; ok {
		return verifyLegacyMetering(p, data, raw, config)
	}
	candidates := []json.RawMessage{}
	add := func(raw json.RawMessage) error {
		var fields map[string]json.RawMessage
		if strictLegacyJSON(raw, &fields, false) != nil || fields == nil {
			return ErrInvalid
		}
		if usage, ok := fields["usage"]; ok {
			candidates = append(candidates, usage)
		}
		return nil
	}
	if raw, ok := meta["lmm_usage"]; ok {
		candidates = append(candidates, raw)
	}
	if raw, ok := result["usage"]; ok {
		candidates = append(candidates, raw)
	}
	if raw := bytes.TrimSpace(result["structuredContent"]); len(raw) > 0 && raw[0] == '{' {
		if add(raw) != nil {
			return LegacyUsageEvidence{}, ErrInvalid
		}
	}
	var content []struct {
		Type string `json:"type"`
		Text string `json:"text"`
	}
	if raw, ok := result["content"]; ok && json.Unmarshal(raw, &content) != nil {
		return LegacyUsageEvidence{}, ErrInvalid
	}
	for _, item := range content {
		text := bytes.TrimSpace([]byte(item.Text))
		if item.Type == "text" && len(text) > 0 && text[0] == '{' && json.Valid(text) {
			if add(text) != nil {
				return LegacyUsageEvidence{}, ErrInvalid
			}
		}
	}
	if len(candidates) == 0 {
		return LegacyUsageEvidence{}, ErrInvalid
	}
	var quantities map[string]int64
	for _, raw := range candidates {
		if len(raw) > 65536 {
			return LegacyUsageEvidence{}, ErrInvalid
		}
		var usage map[string]json.RawMessage
		if strictLegacyJSON(raw, &usage, false) != nil || usage == nil {
			return LegacyUsageEvidence{}, ErrInvalid
		}
		next := map[string]int64{}
		for _, rule := range legacyPricingRules(p) {
			value, ok := usage[rule.Metric]
			var n int64
			if !ok || bytes.Equal(bytes.TrimSpace(value), []byte("null")) || json.Unmarshal(value, &n) != nil {
				return LegacyUsageEvidence{}, ErrInvalid
			}
			next[rule.Metric] = n
		}
		if _, err := LegacyUsageCharge(p, next); err != nil {
			return LegacyUsageEvidence{}, err
		}
		if quantities != nil && !reflect.DeepEqual(quantities, next) {
			return LegacyUsageEvidence{}, ErrConflict
		}
		quantities = next
	}
	return LegacyUsageEvidence{quantities, "tool_reported", append(json.RawMessage(nil), candidates[0]...), digest(data)}, nil
}
func verifyLegacyMetering(p LegacyPricing, data, raw json.RawMessage, cfg *LegacyMeteringConfig) (LegacyUsageEvidence, error) {
	if cfg == nil || cfg.AdapterID == "" || len(cfg.Key) < 32 {
		return LegacyUsageEvidence{}, ErrDenied
	}
	pricingDigest, err := LegacyMeteringPricingDigest(p)
	if err != nil || cfg.Context.PricingDigest != pricingDigest || cfg.Context.CallID == "" || cfg.Context.VersionID == "" || cfg.Context.ToolID == "" || cfg.Context.InputDigest == "" {
		return LegacyUsageEvidence{}, ErrDenied
	}
	allowed := map[string]bool{}
	for _, metric := range cfg.Metrics {
		if _, ok := legacyMetricScales[metric]; !ok || allowed[metric] {
			return LegacyUsageEvidence{}, ErrDenied
		}
		allowed[metric] = true
	}
	for _, rule := range legacyPricingRules(p) {
		if !allowed[rule.Metric] {
			return LegacyUsageEvidence{}, ErrDenied
		}
	}
	var r legacyMeteringReceipt
	if strictLegacyJSON(raw, &r, true) != nil || r.AdapterID != cfg.AdapterID || r.Context != cfg.Context || r.Quantities == nil {
		return LegacyUsageEvidence{}, ErrDenied
	}
	if _, err := LegacyUsageCharge(p, r.Quantities); err != nil {
		return LegacyUsageEvidence{}, err
	}
	resultDigest, err := LegacyMeteringResultDigest(data)
	if err != nil || resultDigest != r.ResultDigest {
		return LegacyUsageEvidence{}, ErrDenied
	}
	message, _ := json.Marshal(struct {
		AdapterID    string                `json:"adapter_id"`
		Context      LegacyMeteringContext `json:"context"`
		Quantities   map[string]int64      `json:"quantities"`
		ResultDigest string                `json:"result_digest"`
	}{r.AdapterID, r.Context, r.Quantities, r.ResultDigest})
	mac := hmac.New(sha256.New, cfg.Key)
	mac.Write([]byte("lmm-tool-market-metering-v1\n"))
	mac.Write(message)
	sig, err := hex.DecodeString(r.Signature)
	if err != nil || len(sig) != sha256.Size || !hmac.Equal(sig, mac.Sum(nil)) {
		return LegacyUsageEvidence{}, ErrDenied
	}
	return LegacyUsageEvidence{r.Quantities, "platform_verified", append(json.RawMessage(nil), raw...), digest(data)}, nil
}
func LegacyMeteringResultDigest(data json.RawMessage) (string, error) {
	var envelope map[string]any
	if strictLegacyJSON(data, &envelope, false) != nil || envelope == nil {
		return "", ErrInvalid
	}
	if meta, ok := envelope["_meta"].(map[string]any); ok {
		delete(meta, "lmm_metering")
		if len(meta) == 0 {
			delete(envelope, "_meta")
		}
	}
	return digest(envelope), nil
}

// Reject duplicate keys before typed decoding so no parser can select a
// different amount. UseNumber preserves large exact values in signed digests.
func strictLegacyJSON(raw []byte, out any, unknown bool) error {
	d := json.NewDecoder(bytes.NewReader(raw))
	d.UseNumber()
	var walk func(int) error
	walk = func(depth int) error {
		if depth > 128 {
			return ErrInvalid
		}
		token, err := d.Token()
		if err != nil {
			return err
		}
		if delim, ok := token.(json.Delim); ok {
			switch delim {
			case '{':
				seen := map[string]bool{}
				for d.More() {
					key, err := d.Token()
					if err != nil {
						return err
					}
					s, ok := key.(string)
					if !ok || seen[s] {
						return ErrInvalid
					}
					seen[s] = true
					if err := walk(depth + 1); err != nil {
						return err
					}
				}
			case '[':
				for d.More() {
					if err := walk(depth + 1); err != nil {
						return err
					}
				}
			default:
				return ErrInvalid
			}
			_, err = d.Token()
			return err
		}
		return nil
	}
	if walk(1) != nil {
		return ErrInvalid
	}
	if _, err := d.Token(); err != io.EOF {
		return ErrInvalid
	}
	d = json.NewDecoder(bytes.NewReader(raw))
	d.UseNumber()
	if unknown {
		d.DisallowUnknownFields()
	}
	return d.Decode(out)
}
