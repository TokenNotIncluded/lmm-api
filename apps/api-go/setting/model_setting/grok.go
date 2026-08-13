package model_setting

// GrokSettings is retained only as a source-compatibility shim for older
// callers. Violation charging no longer reads or registers model-specific
// Grok settings; use operation_setting.ViolationFeeSettings instead.
type GrokSettings struct {
	ViolationDeductionEnabled bool    `json:"violation_deduction_enabled"`
	ViolationDeductionAmount  float64 `json:"violation_deduction_amount"`
}

var defaultGrokSettings = GrokSettings{
	ViolationDeductionEnabled: true,
	ViolationDeductionAmount:  0.05,
}

var grokSettings = defaultGrokSettings

func init() {
	// Intentionally not registered. Existing grok.* database options are
	// ignored after upgrading and cannot select a model-specific fee policy.
}

func GetGrokSettings() *GrokSettings {
	return &grokSettings
}
