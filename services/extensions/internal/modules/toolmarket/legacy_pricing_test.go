// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package toolmarket

import (
	"crypto/hmac"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"reflect"
	"strings"
	"testing"
)

func TestLegacyCreditAmountExactJSON(t *testing.T) {
	for _, raw := range []string{"-1", "9007199254740992", "null", "1.0", "\"1\"", "1e3"} {
		var n CreditAmount
		if json.Unmarshal([]byte(raw), &n) == nil {
			t.Fatalf("accepted %s", raw)
		}
	}
	var n CreditAmount
	if err := json.Unmarshal([]byte("9007199254740991"), &n); err != nil || n != MaxCreditAmount {
		t.Fatal(n, err)
	}
	raw, err := json.Marshal(n)
	if err != nil || string(raw) != "9007199254740991" {
		t.Fatal(string(raw), err)
	}
	if _, err := json.Marshal(CreditAmount(-1)); err == nil {
		t.Fatal("negative output accepted")
	}
}
func TestLegacyPricingNormalizationAndPerRuleCeiling(t *testing.T) {
	p, err := NormalizeLegacyPricing(LegacyPricing{PriceQuota: 17, BillingMode: "fixed"})
	if err != nil || p.BillingMode != "" || p.PriceQuota != 17 {
		t.Fatal(p, err)
	}
	p, err = NormalizeLegacyPricing(LegacyPricing{BillingMode: "input_tokens", InputTokenPriceQuota: 1, MaxInputTokens: 1})
	if err != nil || p.PriceQuota != 1 {
		t.Fatal(p, err)
	}
	p = LegacyPricing{BillingMode: "metered", BillingRules: []LegacyBillingRule{{"input_tokens", 1, 1000000}, {"output_tokens", 1, 1000000}}}
	p, err = NormalizeLegacyPricing(p)
	if err != nil || p.PriceQuota != 2 {
		t.Fatal(p, err)
	}
	amount, err := LegacyUsageCharge(p, map[string]int64{"input_tokens": 1, "output_tokens": 1})
	if err != nil || amount != 2 {
		t.Fatal("must ceil each rule", amount, err)
	}
	for _, quantities := range []map[string]int64{nil, {}, {"input_tokens": 1}, {"input_tokens": -1, "output_tokens": 0}, {"input_tokens": 1000001, "output_tokens": 0}, {"input_tokens": 1, "output_tokens": 0, "images": 0}} {
		if _, err := LegacyUsageCharge(p, quantities); err == nil {
			t.Fatal("bad usage accepted", quantities)
		}
	}
	for _, bad := range []LegacyPricing{
		{BillingMode: "bogus"}, {BillingMode: "fixed", MaxInputTokens: 1},
		{BillingMode: "input_tokens", InputTokenPriceQuota: 1, MaxInputTokens: 1000001},
		{BillingMode: "metered", BillingRules: []LegacyBillingRule{{"unknown", 1, 1}}},
		{BillingMode: "metered", BillingRules: []LegacyBillingRule{{"images", 1, 1}, {"images", 2, 1}}},
		{BillingMode: "metered", BillingRules: []LegacyBillingRule{{"images", MaxCreditAmount, 1000000000000}}},
	} {
		if _, err := NormalizeLegacyPricing(bad); err == nil {
			t.Fatal("bad pricing accepted", bad)
		}
	}
}
func TestLegacyMetricScalesAndBigIntegerIntermediate(t *testing.T) {
	for metric, scale := range legacyMetricScales {
		p := LegacyPricing{BillingMode: "metered", BillingRules: []LegacyBillingRule{{metric, MaxCreditAmount, scale}}}
		n, err := LegacyUsageCharge(p, map[string]int64{metric: scale})
		if err != nil || n != MaxCreditAmount {
			t.Fatal(metric, n, err)
		}
	}
	seller, platform, err := LegacyCreditSplit(MaxCreditAmount, 9999)
	if err != nil || seller+platform != MaxCreditAmount || platform != 9006298534815516 {
		t.Fatal(seller, platform, err)
	}
	if _, _, err := LegacyCreditSplit(1, 10001); err == nil {
		t.Fatal("bad bps accepted")
	}
}
func TestLegacyProviderExactDecimalAndCoverage(t *testing.T) {
	q := LegacyProviderQuote{Provider: "monid", Unit: "call", AmountUSD: "0.000000000000000001", FlatFeeUSD: "0"}
	n, err := LegacyProviderCharge(q, "1", 100, 0)
	if err != nil || n != 1 {
		t.Fatal(n, err)
	}
	q.AmountUSD = "0.000001"
	q.FlatFeeUSD = "0.000001"
	n, err = LegacyProviderCharge(q, "2", 10, 5000)
	if err != nil || n != 2 {
		t.Fatal(n, err)
	}
	if _, err := LegacyProviderCharge(q, "1", 10, 1); err != nil {
		t.Fatal("fee floors to zero", err)
	}
	q.AmountUSD = "1"
	q.FlatFeeUSD = ""
	if _, err := LegacyProviderCharge(q, "1", 1000000, 1); err == nil {
		t.Fatal("seller cost uncovered")
	}
	for _, s := range []string{"NaN", "+1", "1/2", "01", "-1", "1e999", "1000001"} {
		q.AmountUSD = s
		if _, err := LegacyProviderCharge(q, "2", MaxCreditAmount, 0); err == nil {
			t.Fatal("bad decimal", s)
		}
	}
	q.AmountUSD = "1e-2"
	q.Unit = "result"
	if _, err := LegacyProviderCharge(q, "2", MaxCreditAmount, 0); err == nil {
		t.Fatal("variable quote accepted")
	}
	q.Unit = "call"
	if _, err := LegacyProviderCharge(q, "2", 9999, 0); err == nil {
		t.Fatal("ceiling bypass")
	}
}
func legacyUsageTestPrice() LegacyPricing {
	return LegacyPricing{BillingMode: "metered", BillingRules: []LegacyBillingRule{{"input_tokens", 100, 100}}}
}
func TestLegacyUnsignedUsageTrustAndConflicts(t *testing.T) {
	p := legacyUsageTestPrice()
	data := json.RawMessage(`{"structuredContent":{"usage":{"input_tokens":3}},"_meta":{"lmm_usage":{"input_tokens":3}},"content":[{"type":"text","text":"{\"usage\":{\"input_tokens\":3}}"}]}`)
	e, err := ReadLegacyUsage(p, data, nil)
	if err != nil || e.Source != "tool_reported" || e.Quantities["input_tokens"] != 3 {
		t.Fatal(e, err)
	}
	for _, raw := range []string{
		`{"usage":{"input_tokens":1},"usage":{"input_tokens":2}}`,
		`{"usage":{"input_tokens":1,"input_tokens":2}}`,
		`{"usage":{"input_tokens":1},"structuredContent":{"usage":{"input_tokens":2}}}`,
		`{"usage":{"input_tokens":null}}`, `{"usage":{"input_tokens":1.5}}`,
		`{"usage":{"input_tokens":101}}`, `{"usage":{"input_tokens":1}} {}`,
		`{"usage":{}}`, `{"content":[]}`,
	} {
		if _, err := ReadLegacyUsage(p, json.RawMessage(raw), nil); err == nil {
			t.Fatal("bad evidence", raw)
		}
	}
}
func TestLegacySignedUsageBoundToResultAndContext(t *testing.T) {
	p := legacyUsageTestPrice()
	pricingDigest, err := LegacyMeteringPricingDigest(p)
	if err != nil {
		t.Fatal(err)
	}
	cfg := &LegacyMeteringConfig{AdapterID: "collector", Context: LegacyMeteringContext{"call", "version", "tool", "input", pricingDigest}, Key: []byte(strings.Repeat("k", 32)), Metrics: []string{"input_tokens"}}
	base := json.RawMessage(`{"structuredContent":{"value":9007199254740991}}`)
	d, err := LegacyMeteringResultDigest(base)
	if err != nil {
		t.Fatal(err)
	}
	r := legacyMeteringReceipt{AdapterID: cfg.AdapterID, Context: cfg.Context, Quantities: map[string]int64{"input_tokens": 3}, ResultDigest: d}
	message, _ := json.Marshal(struct {
		AdapterID    string                `json:"adapter_id"`
		Context      LegacyMeteringContext `json:"context"`
		Quantities   map[string]int64      `json:"quantities"`
		ResultDigest string                `json:"result_digest"`
	}{r.AdapterID, r.Context, r.Quantities, r.ResultDigest})
	mac := hmac.New(sha256.New, cfg.Key)
	mac.Write([]byte("lmm-tool-market-metering-v1\n"))
	mac.Write(message)
	r.Signature = hex.EncodeToString(mac.Sum(nil))
	var envelope map[string]any
	if err := strictLegacyJSON(base, &envelope, false); err != nil {
		t.Fatal(err)
	}
	envelope["_meta"] = map[string]any{"lmm_metering": r}
	data, _ := json.Marshal(envelope)
	e, err := ReadLegacyUsage(p, data, cfg)
	if err != nil || e.Source != "platform_verified" || !reflect.DeepEqual(e.Quantities, r.Quantities) {
		t.Fatal(e, err)
	}
	badCfg := *cfg
	badCfg.Context.CallID = "other"
	if _, err := ReadLegacyUsage(p, data, &badCfg); err == nil {
		t.Fatal("cross-call receipt")
	}
	badCfg = *cfg
	badCfg.Metrics = []string{"images"}
	if _, err := ReadLegacyUsage(p, data, &badCfg); err == nil {
		t.Fatal("unapproved collector metric")
	}
	changed := p
	changed.BillingRules = append([]LegacyBillingRule(nil), p.BillingRules...)
	changed.BillingRules[0].RateQuota++
	if _, err := ReadLegacyUsage(changed, data, cfg); err == nil {
		t.Fatal("pricing context does not match actual rules")
	}
	if _, err := ReadLegacyUsage(p, data, nil); err == nil {
		t.Fatal("unconfigured receipt accepted")
	}
	envelope["structuredContent"] = map[string]any{"value": 1}
	envelope["usage"] = map[string]int64{"input_tokens": 3}
	tampered, _ := json.Marshal(envelope)
	if _, err := ReadLegacyUsage(p, tampered, cfg); err == nil {
		t.Fatal("tampered result downgraded")
	}
	r.Signature = strings.Repeat("0", 64)
	envelope["_meta"] = map[string]any{"lmm_metering": r}
	tampered, _ = json.Marshal(envelope)
	if _, err := ReadLegacyUsage(p, tampered, cfg); err == nil {
		t.Fatal("bad signature downgraded to unsigned")
	}
}
