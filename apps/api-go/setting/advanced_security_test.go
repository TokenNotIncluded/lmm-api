package setting

import (
	"encoding/json"
	"testing"
)

func TestParseAdvancedSecurityRulesNormalizesAndValidates(t *testing.T) {
	ruleSet, err := ParseAdvancedSecurityRules(`{
  "version": 1,
  "rules": [{
    "id": " prompt-injection ",
    "category": " abuse ",
    "enabled": true,
    "groups": [" default ", "DEFAULT", "premium"],
    "patterns": [" Ignore previous instructions ", "ignore previous instructions"]
  }]
}`)
	if err != nil {
		t.Fatal(err)
	}
	if ruleSet.Version != 1 || len(ruleSet.Rules) != 1 {
		t.Fatalf("unexpected rule set: %+v", ruleSet)
	}
	rule := ruleSet.Rules[0]
	if rule.ID != "prompt-injection" || rule.Name != "prompt-injection" || rule.Category != "abuse" || rule.Severity != "medium" || rule.Source != "local_custom" || rule.Version != "v1" || len(rule.Groups) != 2 || rule.Groups[0] != "default" || rule.Groups[1] != "premium" || len(rule.Patterns) != 1 || rule.Patterns[0] != "Ignore previous instructions" {
		t.Fatalf("unexpected normalized rule: %+v", rule)
	}

	anthropicRuleSet, err := ParseAdvancedSecurityRules(`[{"id":"child","category":"child_safety","enabled":true,"groups":["default"],"patterns":["minor"]}]`)
	if err != nil {
		t.Fatal(err)
	}
	if got := anthropicRuleSet.Rules[0]; got.Severity != "critical" || got.Source != "anthropic_usage_policy" || got.Description == "" {
		t.Fatalf("expected Anthropic-aligned metadata, got %+v", got)
	}

	if _, err := ParseAdvancedSecurityRules(`[{"id":"duplicate","enabled":true,"groups":["default"],"patterns":["x"]},{"id":"duplicate","enabled":true,"groups":["default"],"patterns":["y"]}]`); err == nil {
		t.Fatal("expected duplicate rule ids to be rejected")
	}
	if _, err := ParseAdvancedSecurityRules(`{"version":2,"rules":[]}`); err == nil {
		t.Fatal("expected unsupported rule version to be rejected")
	}
	if _, err := ParseAdvancedSecurityRules(`{"version":1,"rules":[{"id":"empty","enabled":true,"groups":["default"],"patterns":[]}]}`); err == nil {
		t.Fatal("expected empty patterns to be rejected")
	}
	if _, err := ParseAdvancedSecurityRules(`[{"id":"missing-groups","enabled":true,"patterns":["x"]}]`); err == nil {
		t.Fatal("expected rules without explicit groups to be rejected")
	}
	if _, err := ParseAdvancedSecurityRules(`[{"id":"wildcard-group","enabled":true,"groups":["*"],"patterns":["x"]}]`); err == nil {
		t.Fatal("expected wildcard groups to be rejected")
	}
	if _, err := ParseAdvancedSecurityRules(`[{"id":"empty-group","enabled":true,"groups":["   "],"patterns":["x"]}]`); err == nil {
		t.Fatal("expected empty groups to be rejected")
	}
}

func TestAdvancedSecuritySettingsUpdates(t *testing.T) {
	original := GetAdvancedSecuritySettings()
	t.Cleanup(func() {
		SetAdvancedSecurityEnabled(original.Enabled)
		SetAdvancedSecurityOnPrompt(original.OnPrompt)
		_ = UpdateAdvancedSecurityAction(original.Action)
		_ = UpdateAdvancedSecurityRules(AdvancedSecurityRulesToJSONStringForTest(original.RuleSet))
	})

	SetAdvancedSecurityEnabled(true)
	SetAdvancedSecurityOnPrompt(false)
	if err := UpdateAdvancedSecurityAction(AdvancedSecurityActionAudit); err != nil {
		t.Fatal(err)
	}
	if err := UpdateAdvancedSecurityRules(`[{"id":"test","enabled":true,"groups":["default"],"patterns":["needle"]}]`); err != nil {
		t.Fatal(err)
	}

	updated := GetAdvancedSecuritySettings()
	if !updated.Enabled || updated.OnPrompt || updated.Action != AdvancedSecurityActionAudit || len(updated.RuleSet.Rules) != 1 {
		t.Fatalf("unexpected advanced security settings: %+v", updated)
	}
}

func TestApplyAdvancedSecuritySettingsSwapsCompletePolicy(t *testing.T) {
	original := GetAdvancedSecuritySettings()
	t.Cleanup(func() {
		_ = ApplyAdvancedSecuritySettings(
			original.Enabled,
			original.OnPrompt,
			original.Action,
			AdvancedSecurityRulesToJSONStringForTest(original.RuleSet),
		)
	})

	err := ApplyAdvancedSecuritySettings(
		true,
		true,
		AdvancedSecurityActionAudit,
		`[{"id":"disabled","enabled":false,"groups":["default"],"patterns":["needle"]}]`,
	)
	if err != nil {
		t.Fatal(err)
	}
	if ShouldCheckAdvancedSecurityPrompt() {
		t.Fatal("disabled-only rule set should not enable prompt scanning")
	}

	beforeInvalid := GetAdvancedSecuritySettings()
	if err := ApplyAdvancedSecuritySettings(false, false, "invalid", `[{"id":"other","enabled":true,"groups":["default"],"patterns":["other"]}]`); err == nil {
		t.Fatal("expected invalid action to be rejected")
	}
	afterInvalid := GetAdvancedSecuritySettings()
	if afterInvalid.Enabled != beforeInvalid.Enabled || afterInvalid.OnPrompt != beforeInvalid.OnPrompt || afterInvalid.Action != beforeInvalid.Action || len(afterInvalid.RuleSet.Rules) != len(beforeInvalid.RuleSet.Rules) {
		t.Fatalf("invalid policy partially changed runtime settings: before=%+v after=%+v", beforeInvalid, afterInvalid)
	}
}

func TestAdvancedSecurityRiskCategoryCatalogMatchesPolicyLayers(t *testing.T) {
	counts := make(map[string]int)
	for _, category := range GetAdvancedSecurityRiskCategories() {
		counts[category.Layer]++
	}
	if counts["universal_standard"] != 14 {
		t.Fatalf("expected 14 Universal Usage Standards categories, got %d", counts["universal_standard"])
	}
	if counts["high_risk_use_case"] != 7 {
		t.Fatalf("expected 7 high-risk use-case categories, got %d", counts["high_risk_use_case"])
	}
	if counts["additional_guideline"] != 4 {
		t.Fatalf("expected 4 additional guideline categories, got %d", counts["additional_guideline"])
	}
	if counts["custom"] != 1 {
		t.Fatalf("expected one custom category, got %d", counts["custom"])
	}
	if AdvancedSecurityPolicyReferenceDate != "2025-09-15" || AdvancedSecurityPolicyReferenceURL == "" {
		t.Fatalf("unexpected policy reference metadata: %s %s", AdvancedSecurityPolicyReferenceDate, AdvancedSecurityPolicyReferenceURL)
	}
}

func AdvancedSecurityRulesToJSONStringForTest(ruleSet AdvancedSecurityRuleSet) string {
	// Keep test cleanup independent of the current global state.
	encoded, err := json.Marshal(ruleSet)
	if err != nil {
		return `{"version":1,"rules":[]}`
	}
	return string(encoded)
}
