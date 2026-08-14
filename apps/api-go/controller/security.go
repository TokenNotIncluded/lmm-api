package controller

import (
	"bytes"
	"errors"
	"strconv"
	"strings"
	"time"

	"github.com/QuantumNous/new-api/common"
	"github.com/QuantumNous/new-api/dto"
	"github.com/QuantumNous/new-api/model"
	"github.com/QuantumNous/new-api/relaykit/types"
	"github.com/QuantumNous/new-api/setting"
	"github.com/QuantumNous/new-api/setting/operation_setting"

	"github.com/gin-gonic/gin"
)

const publicSecurityStatsWindow = 30 * 24 * time.Hour

func GetPublicSecurityPolicy(c *gin.Context) {
	common.ApiSuccess(c, buildPublicSecurityPolicy())
}

func GetAdminSecurityPolicy(c *gin.Context) {
	settings := setting.GetAdvancedSecuritySettings()
	adminRules := make([]dto.SecurityAdminRule, 0, len(settings.RuleSet.Rules))
	for _, rule := range settings.RuleSet.Rules {
		adminRules = append(adminRules, dto.SecurityAdminRule{
			SecurityRuleSummary: dto.SecurityRuleSummary{
				ID:          rule.ID,
				Name:        rule.Name,
				Category:    rule.Category,
				Layer:       rule.Layer,
				Severity:    rule.Severity,
				Source:      rule.Source,
				Version:     rule.Version,
				Description: rule.Description,
			},
			Enabled:  rule.Enabled,
			Groups:   append([]string(nil), rule.Groups...),
			Patterns: append([]string(nil), rule.Patterns...),
		})
	}
	common.ApiSuccess(c, dto.AdminSecurityPolicy{
		Public: buildPublicSecurityPolicy(),
		Settings: dto.SecuritySettings{
			Enabled:  settings.Enabled,
			OnPrompt: settings.OnPrompt,
			Action:   settings.Action,
		},
		Rules:        adminRules,
		ViolationFee: violationFeeSettingsDTO(),
	})
}

type advancedSecuritySettingsUpdateRequest struct {
	Enabled  *bool             `json:"enabled"`
	OnPrompt *bool             `json:"on_prompt"`
	Action   string            `json:"action"`
	Rules    common.RawMessage `json:"rules"`
}

// UpdateAdvancedSecuritySettings commits the complete guardrail policy in one
// operation so a failed field cannot leave the other fields partially saved.
func UpdateAdvancedSecuritySettings(c *gin.Context) {
	var request advancedSecuritySettingsUpdateRequest
	if err := common.DecodeJson(c.Request.Body, &request); err != nil {
		common.ApiErrorMsg(c, "invalid advanced security settings payload")
		return
	}
	rules := bytes.TrimSpace(request.Rules)
	if request.Enabled == nil || request.OnPrompt == nil || strings.TrimSpace(request.Action) == "" || len(rules) == 0 || (rules[0] != '{' && rules[0] != '[') {
		common.ApiErrorMsg(c, "enabled, on_prompt, action, and rules are required")
		return
	}
	action := strings.ToLower(strings.TrimSpace(request.Action))
	if err := model.UpdateAdvancedSecurityOptions(*request.Enabled, *request.OnPrompt, action, string(rules)); err != nil {
		common.ApiError(c, err)
		return
	}
	recordManageAudit(c, "advanced_security.settings_update", map[string]interface{}{
		"keys": []string{
			setting.AdvancedSecurityEnabledOptionKey,
			setting.AdvancedSecurityOnPromptOptionKey,
			setting.AdvancedSecurityActionOptionKey,
			setting.AdvancedSecurityRulesOptionKey,
		},
	})
	common.ApiSuccess(c, nil)
}

func GetPublicSecurityStats(c *gin.Context) {
	start, end := publicSecurityStatsWindowBounds(c)
	stats, err := model.GetAdvancedSecurityStats(model.AdvancedSecurityEventFilter{
		StartTimestamp: start,
		EndTimestamp:   end,
	})
	if err != nil {
		common.ApiError(c, err)
		return
	}
	common.ApiSuccess(c, buildSecurityStatsDTO(stats, start, end, false))
}

func GetAdminSecurityStats(c *gin.Context) {
	filter, err := parseAdvancedSecurityEventFilter(c, false)
	if err != nil {
		common.ApiErrorMsg(c, err.Error())
		return
	}
	stats, err := model.GetAdvancedSecurityStats(filter)
	if err != nil {
		common.ApiError(c, err)
		return
	}
	common.ApiSuccess(c, buildSecurityStatsDTO(stats, filter.StartTimestamp, filter.EndTimestamp, true))
}

func ListAdminSecurityEvents(c *gin.Context) {
	pageInfo := common.GetPageQuery(c)
	filter, err := parseAdvancedSecurityEventFilter(c, true)
	if err != nil {
		common.ApiErrorMsg(c, err.Error())
		return
	}
	filter.Limit = pageInfo.GetPageSize()
	filter.Offset = pageInfo.GetStartIdx()
	events, total, err := model.ListAdvancedSecurityEvents(filter)
	if err != nil {
		common.ApiError(c, err)
		return
	}
	items := make([]dto.AdvancedSecurityEvent, 0, len(events))
	for _, event := range events {
		items = append(items, dto.AdvancedSecurityEvent{
			ID:            event.ID,
			CreatedAt:     event.CreatedAt,
			RequestID:     event.RequestID,
			UserID:        event.UserID,
			Username:      event.Username,
			TokenID:       event.TokenID,
			ChannelID:     event.ChannelID,
			ModelName:     event.ModelName,
			Group:         event.Group,
			Endpoint:      event.Endpoint,
			Decision:      event.Decision,
			RuleID:        event.RuleID,
			RuleName:      event.RuleName,
			Category:      event.Category,
			Layer:         event.Layer,
			Severity:      event.Severity,
			Source:        event.Source,
			RuleVersion:   event.RuleVersion,
			PatternDigest: event.PatternDigest,
			InputDigest:   event.InputDigest,
			MatchCount:    event.MatchCount,
		})
	}
	pageInfo.SetTotal(int(total))
	pageInfo.SetItems(items)
	common.ApiSuccess(c, pageInfo)
}

func buildPublicSecurityPolicy() dto.PublicSecurityPolicy {
	settings := setting.GetAdvancedSecuritySettings()
	categories := setting.GetAdvancedSecurityRiskCategories()
	categoryDTOs := make([]dto.SecurityRiskCategory, 0, len(categories))
	for _, category := range categories {
		categoryDTOs = append(categoryDTOs, dto.SecurityRiskCategory{
			ID:          category.ID,
			Name:        category.Name,
			Layer:       category.Layer,
			Severity:    category.Severity,
			Description: category.Description,
			Source:      category.Source,
		})
	}

	publicRules := make([]dto.SecurityRuleSummary, 0, len(settings.RuleSet.Rules))
	for _, rule := range settings.RuleSet.Rules {
		if !rule.Enabled {
			continue
		}
		publicRules = append(publicRules, dto.SecurityRuleSummary{
			ID:          rule.ID,
			Name:        rule.Name,
			Category:    rule.Category,
			Layer:       rule.Layer,
			Severity:    rule.Severity,
			Source:      rule.Source,
			Version:     rule.Version,
			Description: rule.Description,
		})
	}

	violationFees := make([]dto.SecurityViolationFeeRule, 0)
	violationSettings := operation_setting.GetViolationFeeSettings()
	if violationSettings != nil {
		for _, policy := range violationSettings.Policies {
			amount := policy.InitialAmountUSD
			if len(policy.AmountsUSD) > 0 {
				amount = policy.AmountsUSD[0]
			}
			violationFees = append(violationFees, dto.SecurityViolationFeeRule{
				Code:              string(types.ErrorCodeViolationFeeUsagePolicy),
				Provider:          "",
				Groups:            append([]string(nil), policy.Groups...),
				Trigger:           "Any upstream usage-policy violation marker, regardless of model or provider.",
				Enabled:           violationSettings.Enabled && policy.Enabled,
				AmountUSD:         amount,
				AmountsUSD:        append([]float64(nil), policy.AmountsUSD...),
				Multiplier:        policy.Multiplier,
				MaxAmountUSD:      policy.MaxAmountUSD,
				PeriodSeconds:     policy.PeriodSeconds,
				ChargeUnit:        "per violating request",
				Retryable:         false,
				Description:       "The configured group policy charges an escalating penalty after a usage-policy violation.",
				ChargingNotes:     "The penalty is deducted from wallet quota only, never below zero. The counter resets after the configured period. Users may appeal and administrators may reverse an approved penalty.",
				LocalGuardrailFee: false,
			})
		}
	}

	return dto.PublicSecurityPolicy{
		PolicyVersion:          setting.AdvancedSecurityPolicyVersion,
		ReferenceEffectiveDate: setting.AdvancedSecurityPolicyReferenceDate,
		ReferenceURL:           setting.AdvancedSecurityPolicyReferenceURL,
		Alignment:              "Anthropic public Usage Policy risk areas, adapted for this relay; not an official equivalent",
		Enforcement: dto.SecuritySettings{
			Enabled:  settings.Enabled,
			OnPrompt: settings.OnPrompt,
			Action:   settings.Action,
		},
		RiskCategories: categoryDTOs,
		Rules:          publicRules,
		ViolationFees:  violationFees,
	}
}

func violationFeeSettingsDTO() dto.SecurityViolationFeeSettings {
	settings := operation_setting.GetViolationFeeSettings()
	result := dto.SecurityViolationFeeSettings{}
	if settings == nil {
		return result
	}
	result.Enabled = settings.Enabled
	result.Policies = make([]dto.SecurityViolationFeePolicy, 0, len(settings.Policies))
	for _, policy := range settings.Policies {
		result.Policies = append(result.Policies, dto.SecurityViolationFeePolicy{
			Name: policy.Name, Groups: append([]string(nil), policy.Groups...), Enabled: policy.Enabled,
			AmountsUSD: append([]float64(nil), policy.AmountsUSD...), InitialAmountUSD: policy.InitialAmountUSD,
			Multiplier: policy.Multiplier, MaxAmountUSD: policy.MaxAmountUSD, PeriodSeconds: policy.PeriodSeconds,
			DrainBalanceWhenShort: policy.DrainBalanceWhenShort,
		})
	}
	return result
}

func buildSecurityStatsDTO(stats model.AdvancedSecurityStats, start, end int64, includeRules bool) dto.SecurityStats {
	result := dto.SecurityStats{
		StartTimestamp:   start,
		EndTimestamp:     end,
		TotalMatches:     stats.TotalMatches,
		BlockedMatches:   stats.BlockedMatches,
		AuditedMatches:   stats.AuditedMatches,
		AffectedRequests: stats.AffectedRequests,
		AffectedUsers:    stats.AffectedUsers,
		ByCategory:       make([]dto.SecurityStatBucket, 0, len(stats.ByCategory)),
	}
	for _, bucket := range stats.ByCategory {
		result.ByCategory = append(result.ByCategory, dto.SecurityStatBucket{Key: bucket.Key, Count: bucket.Count})
	}
	if includeRules {
		result.ByRule = make([]dto.SecurityStatBucket, 0, len(stats.ByRule))
		for _, bucket := range stats.ByRule {
			result.ByRule = append(result.ByRule, dto.SecurityStatBucket{Key: bucket.Key, Count: bucket.Count})
		}
	}
	return result
}

func publicSecurityStatsWindowBounds(c *gin.Context) (int64, int64) {
	now := time.Now().Unix()
	end := now
	start := now - int64(publicSecurityStatsWindow/time.Second)
	if parsed, err := strconv.ParseInt(c.Query("start_timestamp"), 10, 64); err == nil && parsed > 0 {
		start = parsed
	}
	if parsed, err := strconv.ParseInt(c.Query("end_timestamp"), 10, 64); err == nil && parsed > 0 {
		end = parsed
	}
	if end < start {
		return now - int64(publicSecurityStatsWindow/time.Second), now
	}
	maxStart := end - int64((90*24*time.Hour)/time.Second)
	if start < maxStart {
		start = maxStart
	}
	return start, end
}

func parseAdvancedSecurityEventFilter(c *gin.Context, includePaging bool) (model.AdvancedSecurityEventFilter, error) {
	filter := model.AdvancedSecurityEventFilter{}
	var err error
	if value := strings.TrimSpace(c.Query("start_timestamp")); value != "" {
		filter.StartTimestamp, err = strconv.ParseInt(value, 10, 64)
		if err != nil || filter.StartTimestamp < 0 {
			return filter, errors.New("invalid start_timestamp")
		}
	}
	if value := strings.TrimSpace(c.Query("end_timestamp")); value != "" {
		filter.EndTimestamp, err = strconv.ParseInt(value, 10, 64)
		if err != nil || filter.EndTimestamp < 0 {
			return filter, errors.New("invalid end_timestamp")
		}
	}
	if filter.StartTimestamp > 0 && filter.EndTimestamp > 0 && filter.EndTimestamp < filter.StartTimestamp {
		return filter, errors.New("end_timestamp must be greater than or equal to start_timestamp")
	}
	if value := strings.TrimSpace(c.Query("user_id")); value != "" {
		filter.UserID, err = strconv.Atoi(value)
		if err != nil || filter.UserID <= 0 {
			return filter, errors.New("invalid user_id")
		}
	}
	filter.RuleID = strings.TrimSpace(c.Query("rule_id"))
	filter.Category = strings.ToLower(strings.TrimSpace(c.Query("category")))
	filter.ModelName = strings.TrimSpace(c.Query("model_name"))
	filter.Decision = strings.ToLower(strings.TrimSpace(c.Query("decision")))
	if filter.Decision != "" && filter.Decision != model.AdvancedSecurityDecisionBlocked && filter.Decision != model.AdvancedSecurityDecisionAudited {
		return filter, errors.New("decision must be blocked or audited")
	}
	if includePaging {
		pageInfo := common.GetPageQuery(c)
		filter.Limit = pageInfo.GetPageSize()
		filter.Offset = pageInfo.GetStartIdx()
	}
	return filter, nil
}
