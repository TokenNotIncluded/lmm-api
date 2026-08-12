package dto

// SecurityRiskCategory is safe to return to unauthenticated users. It does
// not contain matcher patterns or administrator-only configuration.
type SecurityRiskCategory struct {
	ID          string `json:"id"`
	Name        string `json:"name"`
	Layer       string `json:"layer"`
	Severity    string `json:"severity"`
	Description string `json:"description"`
	Source      string `json:"source"`
}

type SecurityRuleSummary struct {
	ID          string `json:"id"`
	Name        string `json:"name"`
	Category    string `json:"category"`
	Layer       string `json:"layer"`
	Severity    string `json:"severity"`
	Source      string `json:"source"`
	Version     string `json:"version"`
	Description string `json:"description"`
}

type SecurityViolationFeeRule struct {
	Code              string    `json:"code"`
	Provider          string    `json:"provider,omitempty"` // deprecated; policy is no longer provider-specific
	Groups            []string  `json:"groups,omitempty"`
	Trigger           string    `json:"trigger"`
	Enabled           bool      `json:"enabled"`
	AmountUSD         float64   `json:"amount_usd"`
	AmountsUSD        []float64 `json:"amounts_usd,omitempty"`
	Multiplier        float64   `json:"multiplier,omitempty"`
	MaxAmountUSD      float64   `json:"max_amount_usd,omitempty"`
	PeriodSeconds     int64     `json:"period_seconds,omitempty"`
	ChargeUnit        string    `json:"charge_unit"`
	Retryable         bool      `json:"retryable"`
	Description       string    `json:"description"`
	ChargingNotes     string    `json:"charging_notes"`
	LocalGuardrailFee bool      `json:"local_guardrail_fee"`
}

type SecurityViolationFeePolicy struct {
	Name                  string    `json:"name,omitempty"`
	Groups                []string  `json:"groups"`
	Enabled               bool      `json:"enabled"`
	AmountsUSD            []float64 `json:"amounts_usd,omitempty"`
	InitialAmountUSD      float64   `json:"initial_amount_usd"`
	Multiplier            float64   `json:"multiplier"`
	MaxAmountUSD          float64   `json:"max_amount_usd"`
	PeriodSeconds         int64     `json:"period_seconds"`
	DrainBalanceWhenShort bool      `json:"drain_balance_when_short"`
}

type SecurityViolationFeeSettings struct {
	Enabled  bool                         `json:"enabled"`
	Policies []SecurityViolationFeePolicy `json:"policies"`
}

type PublicSecurityPolicy struct {
	PolicyVersion          string                     `json:"policy_version"`
	ReferenceEffectiveDate string                     `json:"reference_effective_date"`
	ReferenceURL           string                     `json:"reference_url"`
	Alignment              string                     `json:"alignment"`
	RiskCategories         []SecurityRiskCategory     `json:"risk_categories"`
	Rules                  []SecurityRuleSummary      `json:"rules"`
	ViolationFees          []SecurityViolationFeeRule `json:"violation_fees"`
}

type SecuritySettings struct {
	Enabled  bool   `json:"enabled"`
	OnPrompt bool   `json:"on_prompt"`
	Action   string `json:"action"`
}

type SecurityAdminRule struct {
	SecurityRuleSummary
	Enabled  bool     `json:"enabled"`
	Patterns []string `json:"patterns"`
}

type AdminSecurityPolicy struct {
	Public       PublicSecurityPolicy         `json:"public"`
	Settings     SecuritySettings             `json:"settings"`
	Rules        []SecurityAdminRule          `json:"rules"`
	ViolationFee SecurityViolationFeeSettings `json:"violation_fee"`
}

type SecurityStatBucket struct {
	Key   string `json:"key"`
	Count int64  `json:"count"`
}

type SecurityStats struct {
	StartTimestamp   int64                `json:"start_timestamp"`
	EndTimestamp     int64                `json:"end_timestamp"`
	TotalMatches     int64                `json:"total_matches"`
	BlockedMatches   int64                `json:"blocked_matches"`
	AuditedMatches   int64                `json:"audited_matches"`
	AffectedRequests int64                `json:"affected_requests"`
	AffectedUsers    int64                `json:"affected_users"`
	ByCategory       []SecurityStatBucket `json:"by_category"`
	ByRule           []SecurityStatBucket `json:"by_rule,omitempty"`
}

type AdvancedSecurityEvent struct {
	ID            uint   `json:"id"`
	CreatedAt     int64  `json:"created_at"`
	RequestID     string `json:"request_id"`
	UserID        int    `json:"user_id"`
	Username      string `json:"username"`
	TokenID       int    `json:"token_id"`
	ChannelID     int    `json:"channel_id"`
	ModelName     string `json:"model_name"`
	Group         string `json:"group"`
	Endpoint      string `json:"endpoint"`
	Decision      string `json:"decision"`
	RuleID        string `json:"rule_id"`
	RuleName      string `json:"rule_name"`
	Category      string `json:"category"`
	Layer         string `json:"layer"`
	Severity      string `json:"severity"`
	Source        string `json:"source"`
	RuleVersion   string `json:"rule_version"`
	PatternDigest string `json:"pattern_digest"`
	InputDigest   string `json:"input_digest"`
	MatchCount    int    `json:"match_count"`
}
