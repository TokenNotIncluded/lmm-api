package setting

import (
	"encoding/json"
	"errors"
	"fmt"
	"strings"
	"sync"
)

const (
	AdvancedSecurityEnabledOptionKey      = "AdvancedSecurityEnabled"
	AdvancedSecurityOnPromptOptionKey     = "AdvancedSecurityOnPromptEnabled"
	AdvancedSecurityActionOptionKey       = "AdvancedSecurityAction"
	AdvancedSecurityRulesOptionKey        = "AdvancedSecurityRules"
	AdvancedSecurityPolicyVersion         = "anthropic-aligned-v1"
	AdvancedSecurityPolicyReferenceDate   = "2025-09-15"
	AdvancedSecurityPolicyReferenceURL    = "https://www.anthropic.com/legal/aup"
	AdvancedSecurityActionBlock           = "block"
	AdvancedSecurityActionAudit           = "audit"
	advancedSecurityRuleSetVersion        = 1
	advancedSecurityMaxRules              = 512
	advancedSecurityMaxPatternsPerRule    = 64
	advancedSecurityMaxPatternLength      = 256
	advancedSecurityMaxRuleIDLength       = 64
	advancedSecurityMaxRuleNameLength     = 128
	advancedSecurityMaxRuleCategoryLength = 64
	advancedSecurityMaxRuleSeverityLength = 16
	advancedSecurityMaxRuleSourceLength   = 64
	advancedSecurityMaxRuleVersionLength  = 32
	advancedSecurityMaxRuleDescriptionLen = 512
	advancedSecurityMaxRuleLayerLength    = 32
	advancedSecurityMaxGroupsPerRule      = 64
	advancedSecurityMaxGroupLength        = 64
)

// AdvancedSecurityRiskCategory describes the public policy taxonomy. The
// taxonomy is aligned with the risk areas Anthropic describes in its public
// Usage Policy, but the actual matcher remains operator-configurable.
type AdvancedSecurityRiskCategory struct {
	ID          string `json:"id"`
	Name        string `json:"name"`
	Layer       string `json:"layer"`
	Severity    string `json:"severity"`
	Description string `json:"description"`
	Source      string `json:"source"`
}

var advancedSecurityRiskCategoryCatalog = []AdvancedSecurityRiskCategory{
	// Universal Usage Standards (the 14 policy headings).
	{ID: "applicable_laws_illegal_activity", Name: "Applicable laws and illegal activity", Layer: "universal_standard", Severity: "high", Description: "Illegal activity, controlled goods, or infringement of third-party rights.", Source: "anthropic_usage_policy"},
	{ID: "critical_infrastructure", Name: "Critical infrastructure", Layer: "universal_standard", Severity: "critical", Description: "Unauthorized access to or disruption of critical systems and services.", Source: "anthropic_usage_policy"},
	{ID: "computer_network_compromise", Name: "Computer and network compromise", Layer: "universal_standard", Severity: "high", Description: "Unauthorized intrusion, malware, destructive cyber activity, or guardrail bypass.", Source: "anthropic_usage_policy"},
	{ID: "weapons", Name: "Weapons and dangerous materials", Layer: "universal_standard", Severity: "critical", Description: "Design, acquisition, or weaponization of harmful weapons or dangerous materials.", Source: "anthropic_usage_policy"},
	{ID: "violence_hate", Name: "Violence and hateful behavior", Layer: "universal_standard", Severity: "high", Description: "Violence, violent extremism, terrorism, intimidation, or hateful behavior.", Source: "anthropic_usage_policy"},
	{ID: "privacy_identity", Name: "Privacy and identity rights", Layer: "universal_standard", Severity: "high", Description: "Unauthorized use of private data, identity misuse, impersonation, or biometric inference.", Source: "anthropic_usage_policy"},
	{ID: "child_safety", Name: "Children's safety", Layer: "universal_standard", Severity: "critical", Description: "Child sexual exploitation, grooming, sextortion, or other abuse of minors.", Source: "anthropic_usage_policy"},
	{ID: "psychological_emotional_harm", Name: "Psychological and emotional harm", Layer: "universal_standard", Severity: "high", Description: "Self-harm, harassment, bullying, emotional abuse, or graphic and gratuitous harm.", Source: "anthropic_usage_policy"},
	{ID: "misinformation", Name: "Misinformation", Layer: "universal_standard", Severity: "high", Description: "Deceptive or misleading information, impersonation, or targeted conspiratorial narratives.", Source: "anthropic_usage_policy"},
	{ID: "democratic_processes_targeted_campaigns", Name: "Democratic processes and targeted campaigns", Layer: "universal_standard", Severity: "high", Description: "Deceptive political influence, vote suppression, or disruption of civic processes.", Source: "anthropic_usage_policy"},
	{ID: "criminal_justice_censorship_surveillance", Name: "Criminal justice, censorship, and surveillance", Layer: "universal_standard", Severity: "critical", Description: "Prohibited high-impact law-enforcement, censorship, surveillance, or biometric uses.", Source: "anthropic_usage_policy"},
	{ID: "fraudulent_abusive_predatory", Name: "Fraudulent, abusive, and predatory practices", Layer: "universal_standard", Severity: "high", Description: "Fraud, scams, spam, predatory practices, deceptive products, or exploitative conduct.", Source: "anthropic_usage_policy"},
	{ID: "platform_abuse", Name: "Platform abuse", Layer: "universal_standard", Severity: "high", Description: "Multi-account evasion, spam automation, ban circumvention, scraping, or jailbreak abuse.", Source: "anthropic_usage_policy"},
	{ID: "sexually_explicit_content", Name: "Sexually explicit content", Layer: "universal_standard", Severity: "high", Description: "Explicit sexual acts, erotic chats, sexual fetishes, incest, or bestiality.", Source: "anthropic_usage_policy"},

	// High-Risk Use Case Requirements.
	{ID: "high_risk_legal", Name: "High-risk: legal", Layer: "high_risk_use_case", Severity: "high", Description: "Legal interpretation, guidance, or decisions with legal implications.", Source: "anthropic_usage_policy"},
	{ID: "high_risk_healthcare", Name: "High-risk: healthcare", Layer: "high_risk_use_case", Severity: "high", Description: "Healthcare decisions, diagnosis, patient care, therapy, or medical guidance.", Source: "anthropic_usage_policy"},
	{ID: "high_risk_insurance", Name: "High-risk: insurance", Layer: "high_risk_use_case", Severity: "high", Description: "Insurance underwriting, claims processing, or coverage decisions.", Source: "anthropic_usage_policy"},
	{ID: "high_risk_finance", Name: "High-risk: finance", Layer: "high_risk_use_case", Severity: "high", Description: "Investment advice, loan approval, or financial eligibility and credit decisions.", Source: "anthropic_usage_policy"},
	{ID: "high_risk_employment_housing", Name: "High-risk: employment and housing", Layer: "high_risk_use_case", Severity: "high", Description: "Hiring, resume screening, employability, housing eligibility, leases, or home loans.", Source: "anthropic_usage_policy"},
	{ID: "high_risk_academic_testing_admissions", Name: "High-risk: academic testing and admissions", Layer: "high_risk_use_case", Severity: "high", Description: "Admissions, standardized testing, certification, or educational institution evaluation.", Source: "anthropic_usage_policy"},
	{ID: "high_risk_media_journalism", Name: "High-risk: media and journalism", Layer: "high_risk_use_case", Severity: "medium", Description: "Automatically generated media or professional journalistic content for external publication.", Source: "anthropic_usage_policy"},

	// Additional Use Case Guidelines.
	{ID: "chatbot_disclosure", Name: "Chatbot disclosure", Layer: "additional_guideline", Severity: "medium", Description: "Consumer-facing chatbots and interactive agents must clearly disclose that users are interacting with AI.", Source: "anthropic_additional_guidelines"},
	{ID: "minors_safety", Name: "Products serving minors", Layer: "additional_guideline", Severity: "high", Description: "Products serving minors require additional age-appropriate safety and privacy controls.", Source: "anthropic_additional_guidelines"},
	{ID: "agentic_use", Name: "Agentic use", Layer: "additional_guideline", Severity: "high", Description: "Agentic systems remain subject to the policy and need controls around delegated actions and tools.", Source: "anthropic_additional_guidelines"},
	{ID: "mcp_server", Name: "Model Context Protocol servers", Layer: "additional_guideline", Severity: "high", Description: "MCP servers and connectors need controls appropriate to their tools, data, and distribution context.", Source: "anthropic_additional_guidelines"},
	{ID: "custom", Name: "Custom operator rule", Layer: "custom", Severity: "medium", Description: "An operator-defined rule outside the standard public taxonomy.", Source: "local_custom"},
}

// AdvancedSecurityRule is an operator-managed literal rule. Patterns are
// matched case-insensitively by the service's Aho-Corasick matcher. Pattern
// values are never returned by the public security policy API.
type AdvancedSecurityRule struct {
	ID          string   `json:"id"`
	Name        string   `json:"name"`
	Category    string   `json:"category"`
	Layer       string   `json:"layer,omitempty"`
	Severity    string   `json:"severity,omitempty"`
	Source      string   `json:"source,omitempty"`
	Version     string   `json:"version,omitempty"`
	Description string   `json:"description,omitempty"`
	Enabled     bool     `json:"enabled"`
	Groups      []string `json:"groups"`
	Patterns    []string `json:"patterns"`
}

type AdvancedSecurityRuleSet struct {
	Version int                    `json:"version"`
	Rules   []AdvancedSecurityRule `json:"rules"`
}

type AdvancedSecuritySettings struct {
	Enabled  bool
	OnPrompt bool
	Action   string
	RuleSet  AdvancedSecurityRuleSet
}

var (
	advancedSecuritySettingsMu sync.RWMutex
	advancedSecuritySettings   = AdvancedSecuritySettings{
		Enabled:  false,
		OnPrompt: true,
		Action:   AdvancedSecurityActionBlock,
		RuleSet: AdvancedSecurityRuleSet{
			Version: advancedSecurityRuleSetVersion,
			Rules:   []AdvancedSecurityRule{},
		},
	}
)

func GetAdvancedSecuritySettings() AdvancedSecuritySettings {
	advancedSecuritySettingsMu.RLock()
	defer advancedSecuritySettingsMu.RUnlock()

	settings := advancedSecuritySettings
	settings.RuleSet.Rules = cloneAdvancedSecurityRules(settings.RuleSet.Rules)
	return settings
}

// GetAdvancedSecurityRiskCategories returns a defensive copy so callers can
// safely expose the catalogue through a JSON response without mutating the
// process-wide policy metadata.
func GetAdvancedSecurityRiskCategories() []AdvancedSecurityRiskCategory {
	categories := make([]AdvancedSecurityRiskCategory, len(advancedSecurityRiskCategoryCatalog))
	copy(categories, advancedSecurityRiskCategoryCatalog)
	return categories
}

func GetAdvancedSecurityRiskCategory(id string) (AdvancedSecurityRiskCategory, bool) {
	id = strings.ToLower(strings.TrimSpace(id))
	for _, category := range advancedSecurityRiskCategoryCatalog {
		if category.ID == id {
			return category, true
		}
	}
	return AdvancedSecurityRiskCategory{}, false
}

func SetAdvancedSecurityEnabled(enabled bool) {
	advancedSecuritySettingsMu.Lock()
	defer advancedSecuritySettingsMu.Unlock()
	advancedSecuritySettings.Enabled = enabled
}

func SetAdvancedSecurityOnPrompt(enabled bool) {
	advancedSecuritySettingsMu.Lock()
	defer advancedSecuritySettingsMu.Unlock()
	advancedSecuritySettings.OnPrompt = enabled
}

func UpdateAdvancedSecurityAction(value string) error {
	action, err := normalizeAdvancedSecurityAction(value)
	if err != nil {
		return err
	}

	advancedSecuritySettingsMu.Lock()
	defer advancedSecuritySettingsMu.Unlock()
	advancedSecuritySettings.Action = action
	return nil
}

func UpdateAdvancedSecurityRules(value string) error {
	ruleSet, err := ParseAdvancedSecurityRules(value)
	if err != nil {
		return err
	}

	advancedSecuritySettingsMu.Lock()
	defer advancedSecuritySettingsMu.Unlock()
	advancedSecuritySettings.RuleSet = ruleSet
	return nil
}

// ApplyAdvancedSecuritySettings validates and swaps the complete guardrail
// configuration under one lock. Callers that persist the four related option
// keys together can therefore avoid exposing a partially updated runtime
// policy to concurrent requests.
func ApplyAdvancedSecuritySettings(enabled, onPrompt bool, actionValue, rulesValue string) error {
	action, err := normalizeAdvancedSecurityAction(actionValue)
	if err != nil {
		return err
	}
	ruleSet, err := ParseAdvancedSecurityRules(rulesValue)
	if err != nil {
		return err
	}

	advancedSecuritySettingsMu.Lock()
	defer advancedSecuritySettingsMu.Unlock()
	advancedSecuritySettings = AdvancedSecuritySettings{
		Enabled:  enabled,
		OnPrompt: onPrompt,
		Action:   action,
		RuleSet:  ruleSet,
	}
	return nil
}

func normalizeAdvancedSecurityAction(value string) (string, error) {
	action := strings.ToLower(strings.TrimSpace(value))
	if action != AdvancedSecurityActionBlock && action != AdvancedSecurityActionAudit {
		return "", fmt.Errorf("advanced security action must be %q or %q", AdvancedSecurityActionBlock, AdvancedSecurityActionAudit)
	}
	return action, nil
}

func ShouldCheckAdvancedSecurityPrompt() bool {
	advancedSecuritySettingsMu.RLock()
	defer advancedSecuritySettingsMu.RUnlock()
	if !advancedSecuritySettings.Enabled || !advancedSecuritySettings.OnPrompt {
		return false
	}
	for _, rule := range advancedSecuritySettings.RuleSet.Rules {
		if rule.Enabled && len(rule.Patterns) > 0 && len(rule.Groups) > 0 {
			return true
		}
	}
	return false
}

// AdvancedSecurityRuleAppliesToGroup deliberately requires an explicit,
// non-empty request group. A rule without a configured group never matches;
// there is no wildcard or global fallback.
func AdvancedSecurityRuleAppliesToGroup(rule AdvancedSecurityRule, group string) bool {
	group = strings.TrimSpace(group)
	if group == "" {
		return false
	}
	for _, configured := range rule.Groups {
		if strings.EqualFold(strings.TrimSpace(configured), group) {
			return true
		}
	}
	return false
}

func AdvancedSecurityRulesToJSONString() string {
	settings := GetAdvancedSecuritySettings()
	encoded, err := json.Marshal(settings.RuleSet)
	if err != nil {
		return `{"version":1,"rules":[]}`
	}
	return string(encoded)
}

func ParseAdvancedSecurityRules(value string) (AdvancedSecurityRuleSet, error) {
	raw := strings.TrimSpace(value)
	if raw == "" {
		return emptyAdvancedSecurityRuleSet(), nil
	}

	var ruleSet AdvancedSecurityRuleSet
	if strings.HasPrefix(raw, "[") {
		if err := json.Unmarshal([]byte(raw), &ruleSet.Rules); err != nil {
			return AdvancedSecurityRuleSet{}, fmt.Errorf("advanced security rules must be valid JSON: %w", err)
		}
		ruleSet.Version = advancedSecurityRuleSetVersion
	} else if err := json.Unmarshal([]byte(raw), &ruleSet); err != nil {
		return AdvancedSecurityRuleSet{}, fmt.Errorf("advanced security rules must be valid JSON: %w", err)
	}

	if ruleSet.Version == 0 {
		ruleSet.Version = advancedSecurityRuleSetVersion
	}
	if ruleSet.Version != advancedSecurityRuleSetVersion {
		return AdvancedSecurityRuleSet{}, fmt.Errorf("unsupported advanced security rule version: %d", ruleSet.Version)
	}
	if len(ruleSet.Rules) > advancedSecurityMaxRules {
		return AdvancedSecurityRuleSet{}, fmt.Errorf("advanced security rule count cannot exceed %d", advancedSecurityMaxRules)
	}

	seenIDs := make(map[string]struct{}, len(ruleSet.Rules))
	for index := range ruleSet.Rules {
		rule := &ruleSet.Rules[index]
		rule.ID = strings.TrimSpace(rule.ID)
		rule.Name = strings.TrimSpace(rule.Name)
		rule.Category = strings.ToLower(strings.TrimSpace(rule.Category))
		if rule.ID == "" || len(rule.ID) > advancedSecurityMaxRuleIDLength {
			return AdvancedSecurityRuleSet{}, fmt.Errorf("advanced security rule %d has an invalid id", index+1)
		}
		if _, exists := seenIDs[rule.ID]; exists {
			return AdvancedSecurityRuleSet{}, fmt.Errorf("advanced security rule id %q is duplicated", rule.ID)
		}
		seenIDs[rule.ID] = struct{}{}
		if rule.Name == "" {
			rule.Name = rule.ID
		}
		if len(rule.Name) > advancedSecurityMaxRuleNameLength {
			return AdvancedSecurityRuleSet{}, fmt.Errorf("advanced security rule %q has a name that is too long", rule.ID)
		}
		if rule.Category == "" {
			rule.Category = "custom"
		}
		if len(rule.Category) > advancedSecurityMaxRuleCategoryLength {
			return AdvancedSecurityRuleSet{}, fmt.Errorf("advanced security rule %q has a category that is too long", rule.ID)
		}
		if len(strings.TrimSpace(rule.Layer)) > advancedSecurityMaxRuleLayerLength {
			return AdvancedSecurityRuleSet{}, fmt.Errorf("advanced security rule %q has a layer that is too long", rule.ID)
		}
		if len(strings.TrimSpace(rule.Severity)) > advancedSecurityMaxRuleSeverityLength {
			return AdvancedSecurityRuleSet{}, fmt.Errorf("advanced security rule %q has a severity that is too long", rule.ID)
		}
		if len(strings.TrimSpace(rule.Source)) > advancedSecurityMaxRuleSourceLength {
			return AdvancedSecurityRuleSet{}, fmt.Errorf("advanced security rule %q has a source that is too long", rule.ID)
		}
		if len(strings.TrimSpace(rule.Version)) > advancedSecurityMaxRuleVersionLength {
			return AdvancedSecurityRuleSet{}, fmt.Errorf("advanced security rule %q has a version that is too long", rule.ID)
		}
		if len(strings.TrimSpace(rule.Description)) > advancedSecurityMaxRuleDescriptionLen {
			return AdvancedSecurityRuleSet{}, fmt.Errorf("advanced security rule %q has a description that is too long", rule.ID)
		}
		if len(rule.Patterns) == 0 || len(rule.Patterns) > advancedSecurityMaxPatternsPerRule {
			return AdvancedSecurityRuleSet{}, fmt.Errorf("advanced security rule %q must contain 1-%d patterns", rule.ID, advancedSecurityMaxPatternsPerRule)
		}
		groups, err := normalizeAdvancedSecurityGroups(rule.Groups)
		if err != nil {
			return AdvancedSecurityRuleSet{}, fmt.Errorf("advanced security rule %q: %w", rule.ID, err)
		}
		rule.Groups = groups
		applyAdvancedSecurityRuleMetadata(rule)
		switch rule.Severity {
		case "low", "medium", "high", "critical":
		default:
			return AdvancedSecurityRuleSet{}, fmt.Errorf("advanced security rule %q has an invalid severity", rule.ID)
		}

		seenPatterns := make(map[string]struct{}, len(rule.Patterns))
		patterns := make([]string, 0, len(rule.Patterns))
		for _, pattern := range rule.Patterns {
			pattern = strings.TrimSpace(pattern)
			if pattern == "" || len(pattern) > advancedSecurityMaxPatternLength {
				return AdvancedSecurityRuleSet{}, fmt.Errorf("advanced security rule %q contains an invalid pattern", rule.ID)
			}
			normalized := strings.ToLower(pattern)
			if _, exists := seenPatterns[normalized]; exists {
				continue
			}
			seenPatterns[normalized] = struct{}{}
			patterns = append(patterns, pattern)
		}
		if len(patterns) == 0 {
			return AdvancedSecurityRuleSet{}, fmt.Errorf("advanced security rule %q must contain a non-empty pattern", rule.ID)
		}
		rule.Patterns = patterns
	}

	return ruleSet, nil
}

func normalizeAdvancedSecurityGroups(groups []string) ([]string, error) {
	if len(groups) == 0 || len(groups) > advancedSecurityMaxGroupsPerRule {
		return nil, fmt.Errorf("groups must contain 1-%d explicit group names", advancedSecurityMaxGroupsPerRule)
	}
	seen := make(map[string]struct{}, len(groups))
	result := make([]string, 0, len(groups))
	for _, group := range groups {
		group = strings.TrimSpace(group)
		if group == "" || len(group) > advancedSecurityMaxGroupLength {
			return nil, fmt.Errorf("groups must contain non-empty names no longer than %d characters", advancedSecurityMaxGroupLength)
		}
		if group == "*" {
			return nil, errors.New("wildcard groups are not allowed; list each group explicitly")
		}
		key := strings.ToLower(group)
		if _, exists := seen[key]; exists {
			continue
		}
		seen[key] = struct{}{}
		result = append(result, group)
	}
	if len(result) == 0 {
		return nil, errors.New("groups must contain at least one explicit group name")
	}
	return result, nil
}

func applyAdvancedSecurityRuleMetadata(rule *AdvancedSecurityRule) {
	if rule == nil {
		return
	}
	category, ok := GetAdvancedSecurityRiskCategory(rule.Category)
	if !ok {
		category, _ = GetAdvancedSecurityRiskCategory("custom")
	}
	if strings.TrimSpace(rule.Severity) == "" {
		rule.Severity = category.Severity
	} else {
		rule.Severity = strings.ToLower(strings.TrimSpace(rule.Severity))
	}
	if strings.TrimSpace(rule.Layer) == "" {
		rule.Layer = category.Layer
	} else {
		rule.Layer = strings.ToLower(strings.TrimSpace(rule.Layer))
	}
	if strings.TrimSpace(rule.Source) == "" {
		rule.Source = category.Source
	} else {
		rule.Source = strings.TrimSpace(rule.Source)
	}
	if strings.TrimSpace(rule.Version) == "" {
		rule.Version = "v1"
	} else {
		rule.Version = strings.TrimSpace(rule.Version)
	}
	if strings.TrimSpace(rule.Description) == "" {
		rule.Description = category.Description
	} else {
		rule.Description = strings.TrimSpace(rule.Description)
	}
}

func ValidateAdvancedSecurityOption(key string, value string) error {
	switch key {
	case AdvancedSecurityActionOptionKey:
		return validateAdvancedSecurityAction(value)
	case AdvancedSecurityRulesOptionKey:
		_, err := ParseAdvancedSecurityRules(value)
		return err
	}
	return nil
}

func validateAdvancedSecurityAction(value string) error {
	action := strings.ToLower(strings.TrimSpace(value))
	if action != AdvancedSecurityActionBlock && action != AdvancedSecurityActionAudit {
		return errors.New("advanced security action must be block or audit")
	}
	return nil
}

func emptyAdvancedSecurityRuleSet() AdvancedSecurityRuleSet {
	return AdvancedSecurityRuleSet{
		Version: advancedSecurityRuleSetVersion,
		Rules:   []AdvancedSecurityRule{},
	}
}

func cloneAdvancedSecurityRules(rules []AdvancedSecurityRule) []AdvancedSecurityRule {
	cloned := make([]AdvancedSecurityRule, len(rules))
	for index, rule := range rules {
		cloned[index] = rule
		cloned[index].Groups = append([]string(nil), rule.Groups...)
		cloned[index].Patterns = append([]string(nil), rule.Patterns...)
	}
	return cloned
}
