package controller

import (
	"archive/zip"
	"bytes"
	"encoding/json"
	"fmt"
	"net/http"
	"net/url"
	"strconv"
	"strings"
	"time"

	"github.com/QuantumNous/new-api/common"
	"github.com/QuantumNous/new-api/model"
	"github.com/gin-gonic/gin"
	"gorm.io/gorm"
)

const (
	financeExportDefaultWindow = 30 * 24 * 60 * 60
	// Keep a generous one-year upper bound while retaining a hard server-side
	// limit. The UI accepts an exact datetime range; this bound prevents an
	// accidental all-history export from exhausting the log database.
	financeExportMaxWindow     = 365 * 24 * 60 * 60
	financeExportMaxRows       = 200_000
)

// FinanceExport is deliberately an allowlisted projection. Never add a
// password, access token, channel key, provider payload, request body, IP,
// or opaque `other` field here: this endpoint is designed for AI analysis and
// the resulting files may leave the administrator's browser.
type financeExportManifest struct {
	SchemaVersion  string          `json:"schema_version"`
	GeneratedAt    int64           `json:"generated_at"`
	StartTimestamp int64           `json:"start_timestamp"`
	EndTimestamp   int64           `json:"end_timestamp"`
	Redactions     []string        `json:"redactions"`
	Rows           map[string]int  `json:"rows"`
	Truncated      map[string]bool `json:"truncated"`
	Notes          []string        `json:"notes"`
}

type financeUserExport struct {
	ID                 int      `json:"user_id" gorm:"column:id"`
	Username           string   `json:"username" gorm:"column:username"`
	Role               int      `json:"role" gorm:"column:role"`
	Status             int      `json:"status" gorm:"column:status"`
	Group              string   `json:"group" gorm:"column:group"`
	Quota              int      `json:"quota" gorm:"column:quota"`
	UsedQuota          int      `json:"used_quota" gorm:"column:used_quota"`
	AffQuota           int      `json:"affiliate_quota" gorm:"column:aff_quota"`
	AffHistoryQuota    int      `json:"affiliate_history_quota" gorm:"column:aff_history"`
	RequestCount       int      `json:"request_count" gorm:"column:request_count"`
	CreatedAt          int64    `json:"created_at" gorm:"column:created_at"`
	LastAPIActivityAt  int64    `json:"last_api_activity_at" gorm:"column:last_api_activity_at"`
	TrustLevelOverride *int     `json:"trust_level_override,omitempty" gorm:"column:trust_level_override"`
	GroupRatio         *float64 `json:"effective_group_ratio,omitempty" gorm:"-"`
	TopupGroupRatio    *float64 `json:"effective_topup_group_ratio,omitempty" gorm:"-"`
}

type financeChannelExport struct {
	ID                 int     `json:"channel_id" gorm:"column:id"`
	Type               int     `json:"type" gorm:"column:type"`
	Status             int     `json:"status" gorm:"column:status"`
	Name               string  `json:"name" gorm:"column:name"`
	Weight             *uint   `json:"weight,omitempty" gorm:"column:weight"`
	CreatedTime        int64   `json:"created_time" gorm:"column:created_time"`
	TestTime           int64   `json:"test_time" gorm:"column:test_time"`
	ResponseTime       int     `json:"response_time" gorm:"column:response_time"`
	BaseURL            *string `json:"base_url,omitempty" gorm:"column:base_url"`
	Balance            float64 `json:"balance" gorm:"column:balance"`
	BalanceUpdatedTime int64   `json:"balance_updated_time" gorm:"column:balance_updated_time"`
	Models             string  `json:"models" gorm:"column:models"`
	ModelMapping       *string `json:"model_mapping,omitempty" gorm:"column:model_mapping"`
	Group              string  `json:"group" gorm:"column:group"`
	UsedQuota          int64   `json:"used_quota" gorm:"column:used_quota"`
	Priority           *int64  `json:"priority,omitempty" gorm:"column:priority"`
	AutoBan            *int    `json:"auto_ban,omitempty" gorm:"column:auto_ban"`
	Tag                *string `json:"tag,omitempty" gorm:"column:tag"`
}

type financePlanExport struct {
	ID                   int     `json:"plan_id" gorm:"column:id"`
	Title                string  `json:"title" gorm:"column:title"`
	Subtitle             string  `json:"subtitle" gorm:"column:subtitle"`
	PriceAmount          float64 `json:"price_amount" gorm:"column:price_amount"`
	Currency             string  `json:"currency" gorm:"column:currency"`
	DurationUnit         string  `json:"duration_unit" gorm:"column:duration_unit"`
	DurationValue        int     `json:"duration_value" gorm:"column:duration_value"`
	CustomSeconds        int64   `json:"custom_seconds" gorm:"column:custom_seconds"`
	Enabled              bool    `json:"enabled" gorm:"column:enabled"`
	SortOrder            int     `json:"sort_order" gorm:"column:sort_order"`
	AllowBalancePay      *bool   `json:"allow_balance_pay,omitempty" gorm:"column:allow_balance_pay"`
	AllowWalletOverflow  *bool   `json:"allow_wallet_overflow,omitempty" gorm:"column:allow_wallet_overflow"`
	MaxPurchasePerUser   int     `json:"max_purchase_per_user" gorm:"column:max_purchase_per_user"`
	UpgradeGroup         string  `json:"upgrade_group" gorm:"column:upgrade_group"`
	DowngradeGroup       string  `json:"downgrade_group" gorm:"column:downgrade_group"`
	TotalAmount          int64   `json:"total_amount" gorm:"column:total_amount"`
	QuotaResetPeriod     string  `json:"quota_reset_period" gorm:"column:quota_reset_period"`
	QuotaResetCustomSecs int64   `json:"quota_reset_custom_seconds" gorm:"column:quota_reset_custom_seconds"`
	CreatedAt            int64   `json:"created_at" gorm:"column:created_at"`
	UpdatedAt            int64   `json:"updated_at" gorm:"column:updated_at"`
}

type financeTopUpExport struct {
	ID                   int     `json:"topup_id" gorm:"column:id"`
	UserID               int     `json:"user_id" gorm:"column:user_id"`
	Amount               int64   `json:"amount" gorm:"column:amount"`
	CreditedQuota        int64   `json:"credited_quota" gorm:"column:credited_quota"`
	ExpectedAmountMicros int64   `json:"expected_amount_micros" gorm:"column:expected_amount_micros"`
	SettledAmountMicros  int64   `json:"settled_amount_micros" gorm:"column:settled_amount_micros"`
	SettlementCurrency   string  `json:"settlement_currency" gorm:"column:settlement_currency"`
	Money                float64 `json:"money" gorm:"column:money"`
	PaymentMethod        string  `json:"payment_method" gorm:"column:payment_method"`
	PaymentProvider      string  `json:"payment_provider" gorm:"column:payment_provider"`
	CreateTime           int64   `json:"create_time" gorm:"column:create_time"`
	CompleteTime         int64   `json:"complete_time" gorm:"column:complete_time"`
	Status               string  `json:"status" gorm:"column:status"`
}

type financeSubscriptionOrderExport struct {
	ID              int     `json:"order_id" gorm:"column:id"`
	UserID          int     `json:"user_id" gorm:"column:user_id"`
	PlanID          int     `json:"plan_id" gorm:"column:plan_id"`
	Money           float64 `json:"money" gorm:"column:money"`
	PaymentMethod   string  `json:"payment_method" gorm:"column:payment_method"`
	PaymentProvider string  `json:"payment_provider" gorm:"column:payment_provider"`
	Status          string  `json:"status" gorm:"column:status"`
	CreateTime      int64   `json:"create_time" gorm:"column:create_time"`
	CompleteTime    int64   `json:"complete_time" gorm:"column:complete_time"`
}

type financeUsageExport struct {
	ID               int    `json:"log_id" gorm:"column:id"`
	UserID           int    `json:"user_id" gorm:"column:user_id"`
	CreatedAt        int64  `json:"created_at" gorm:"column:created_at"`
	Type             int    `json:"type" gorm:"column:type"`
	Username         string `json:"username" gorm:"column:username"`
	TokenName        string `json:"token_name" gorm:"column:token_name"`
	ModelName        string `json:"model_name" gorm:"column:model_name"`
	Quota            int    `json:"quota" gorm:"column:quota"`
	PromptTokens     int    `json:"prompt_tokens" gorm:"column:prompt_tokens"`
	CompletionTokens int    `json:"completion_tokens" gorm:"column:completion_tokens"`
	UseTime          int    `json:"use_time" gorm:"column:use_time"`
	IsStream         bool   `json:"is_stream" gorm:"column:is_stream"`
	ChannelID        int    `json:"channel_id" gorm:"column:channel_id"`
	Group            string `json:"group" gorm:"column:group"`
}

type financeBountyLedgerExport struct {
	ID                 int    `json:"ledger_id" gorm:"column:id"`
	ProjectID          int    `json:"project_id" gorm:"column:project_id"`
	ChallengeID        int    `json:"challenge_id" gorm:"column:challenge_id"`
	UserID             int    `json:"user_id" gorm:"column:user_id"`
	CounterpartyUserID int    `json:"counterparty_user_id" gorm:"column:counterparty_user_id"`
	Kind               string `json:"kind" gorm:"column:kind"`
	Quota              int    `json:"quota" gorm:"column:quota"`
	Note               string `json:"note" gorm:"column:note"`
	CreatedAt          int64  `json:"created_at" gorm:"column:created_at"`
}

type financeCheckinExport struct {
	ID           int    `json:"checkin_id" gorm:"column:id"`
	UserID       int    `json:"user_id" gorm:"column:user_id"`
	CheckinDate  string `json:"checkin_date" gorm:"column:checkin_date"`
	QuotaAwarded int    `json:"quota_awarded" gorm:"column:quota_awarded"`
	CreatedAt    int64  `json:"created_at" gorm:"column:created_at"`
}

type financeRedemptionExport struct {
	ID           int    `json:"redemption_id" gorm:"column:id"`
	UserID       int    `json:"created_by_user_id" gorm:"column:user_id"`
	Status       int    `json:"status" gorm:"column:status"`
	Name         string `json:"name" gorm:"column:name"`
	Quota        int    `json:"quota" gorm:"column:quota"`
	CreatedTime  int64  `json:"created_time" gorm:"column:created_time"`
	RedeemedTime int64  `json:"redeemed_time" gorm:"column:redeemed_time"`
	UsedUserID   int    `json:"used_user_id" gorm:"column:used_user_id"`
	ExpiredTime  int64  `json:"expired_time" gorm:"column:expired_time"`
}

type financeUserSubscriptionExport struct {
	ID                  int    `json:"subscription_id" gorm:"column:id"`
	UserID              int    `json:"user_id" gorm:"column:user_id"`
	PlanID              int    `json:"plan_id" gorm:"column:plan_id"`
	AmountTotal         int64  `json:"amount_total" gorm:"column:amount_total"`
	AmountUsed          int64  `json:"amount_used" gorm:"column:amount_used"`
	StartTime           int64  `json:"start_time" gorm:"column:start_time"`
	EndTime             int64  `json:"end_time" gorm:"column:end_time"`
	Status              string `json:"status" gorm:"column:status"`
	Source              string `json:"source" gorm:"column:source"`
	LastResetTime       int64  `json:"last_reset_time" gorm:"column:last_reset_time"`
	NextResetTime       int64  `json:"next_reset_time" gorm:"column:next_reset_time"`
	UpgradeGroup        string `json:"upgrade_group" gorm:"column:upgrade_group"`
	PrevUserGroup       string `json:"previous_user_group" gorm:"column:prev_user_group"`
	DowngradeGroup      string `json:"downgrade_group" gorm:"column:downgrade_group"`
	AllowWalletOverflow bool   `json:"allow_wallet_overflow" gorm:"column:allow_wallet_overflow"`
	CreatedAt           int64  `json:"created_at" gorm:"column:created_at"`
	UpdatedAt           int64  `json:"updated_at" gorm:"column:updated_at"`
}

type financeExportBundle struct {
	Manifest           financeExportManifest
	Options            map[string]string
	EffectivePricing   []model.Pricing
	Users              []financeUserExport
	Channels           []financeChannelExport
	Plans              []financePlanExport
	TopUps             []financeTopUpExport
	SubscriptionOrders []financeSubscriptionOrderExport
	Usage              []financeUsageExport
	BountyLedger       []financeBountyLedgerExport
	Checkins           []financeCheckinExport
	Redemptions        []financeRedemptionExport
	UserSubscriptions  []financeUserSubscriptionExport
}

func parseFinanceExportWindow(c *gin.Context) (int64, int64, error) {
	now := time.Now().Unix()
	start := now - financeExportDefaultWindow
	end := now
	parse := func(name string, fallback int64) (int64, error) {
		value := strings.TrimSpace(c.Query(name))
		if value == "" {
			return fallback, nil
		}
		parsed, err := strconv.ParseInt(value, 10, 64)
		if err != nil || parsed <= 0 {
			return 0, fmt.Errorf("invalid %s", name)
		}
		return parsed, nil
	}
	var err error
	if start, err = parse("start_timestamp", start); err != nil {
		return 0, 0, err
	}
	if end, err = parse("end_timestamp", end); err != nil {
		return 0, 0, err
	}
	if start >= end {
		return 0, 0, fmt.Errorf("start_timestamp must be before end_timestamp")
	}
	if end-start > financeExportMaxWindow {
		return 0, 0, fmt.Errorf("export window cannot exceed %d days", financeExportMaxWindow/(24*60*60))
	}
	return start, end, nil
}

var financeExportOptionKeys = map[string]struct{}{
	"ModelPrice":                        {},
	"ModelRatio":                        {},
	"CacheRatio":                        {},
	"CreateCacheRatio":                  {},
	"CompletionRatio":                   {},
	"ImageRatio":                        {},
	"AudioRatio":                        {},
	"AudioCompletionRatio":              {},
	"GroupRatio":                        {},
	"GroupGroupRatio":                   {},
	"TopupGroupRatio":                   {},
	"UserUsableGroups":                  {},
	"QuotaPerUnit":                      {},
	"Price":                             {},
	"USDExchangeRate":                   {},
	"MinTopUp":                          {},
	"DataExportInterval":                {},
	"payment_setting.amount_options":    {},
	"payment_setting.amount_discount":   {},
	"tool_price_setting.prices":         {},
	"billing_setting.billing_mode":      {},
	"billing_setting.billing_expr":      {},
	"checkin_setting.enabled":           {},
	"checkin_setting.min_quota":         {},
	"checkin_setting.max_quota":         {},
	"checkin_setting.level_multipliers": {},
}

func loadFinanceExportBundle(start, end int64) (financeExportBundle, error) {
	bundle := financeExportBundle{
		Options: make(map[string]string),
		Manifest: financeExportManifest{
			SchemaVersion:  "lmm-finance-export/v1",
			GeneratedAt:    time.Now().Unix(),
			StartTimestamp: start,
			EndTimestamp:   end,
			Redactions: []string{
				"user passwords, access tokens, API keys, channel keys, redemption keys, provider payloads, IPs, request bodies, opaque log fields, channel remarks, URL credentials/query strings, trade/provider event identifiers",
			},
			Rows:      make(map[string]int),
			Truncated: make(map[string]bool),
			Notes: []string{
				"usage, top-up, subscription-order, check-in, and bounty-ledger rows are limited to the requested time window",
				"channels-pricing includes configured channel balances, model lists, and model mappings; it does not make live upstream requests",
				"redemptions exclude redemption keys and include all non-deleted codes subject to the row limit",
				"user-subscriptions contains quota entitlement snapshots; payment trade/provider payloads remain excluded",
				"the export is an analysis snapshot, not an accounting ledger of record",
			},
		},
	}
	if model.DB == nil || model.LOG_DB == nil {
		return bundle, gorm.ErrInvalidDB
	}

	options, err := model.AllOption()
	if err != nil {
		return bundle, err
	}
	for _, option := range options {
		if _, ok := financeExportOptionKeys[option.Key]; ok {
			bundle.Options[option.Key] = option.Value
		}
	}
	bundle.EffectivePricing = model.GetPricing()

	if err := model.DB.Model(&model.User{}).
		Select("id", "username", "role", "status", "group", "quota", "used_quota", "aff_quota", "aff_history", "request_count", "created_at", "last_api_activity_at", "trust_level_override").
		Order("id ASC").Find(&bundle.Users).Error; err != nil {
		return bundle, err
	}
	applyFinanceUserRatios(bundle.Users, bundle.Options)
	if err := model.DB.Model(&model.Channel{}).
		Select("id", "type", "status", "name", "weight", "created_time", "test_time", "response_time", "base_url", "balance", "balance_updated_time", "models", "model_mapping", "group", "used_quota", "priority", "auto_ban", "tag").
		Order("id ASC").Find(&bundle.Channels).Error; err != nil {
		return bundle, err
	}
	for index := range bundle.Channels {
		bundle.Channels[index].BaseURL = sanitizeFinanceBaseURL(bundle.Channels[index].BaseURL)
	}
	if err := model.DB.Model(&model.SubscriptionPlan{}).
		Select("id", "title", "subtitle", "price_amount", "currency", "duration_unit", "duration_value", "custom_seconds", "enabled", "sort_order", "allow_balance_pay", "allow_wallet_overflow", "max_purchase_per_user", "upgrade_group", "downgrade_group", "total_amount", "quota_reset_period", "quota_reset_custom_seconds", "created_at", "updated_at").
		Order("sort_order ASC, id ASC").Find(&bundle.Plans).Error; err != nil {
		return bundle, err
	}
	if err := model.DB.Model(&model.TopUp{}).
		Select("id", "user_id", "amount", "credited_quota", "expected_amount_micros", "settled_amount_micros", "settlement_currency", "money", "payment_method", "payment_provider", "create_time", "complete_time", "status").
		Where("create_time >= ? AND create_time <= ?", start, end).
		Order("create_time ASC, id ASC").Limit(financeExportMaxRows).Find(&bundle.TopUps).Error; err != nil {
		return bundle, err
	}
	if err := model.DB.Model(&model.SubscriptionOrder{}).
		Select("id", "user_id", "plan_id", "money", "payment_method", "payment_provider", "status", "create_time", "complete_time").
		Where("create_time >= ? AND create_time <= ?", start, end).
		Order("create_time ASC, id ASC").Limit(financeExportMaxRows).Find(&bundle.SubscriptionOrders).Error; err != nil {
		return bundle, err
	}
	usageTypes := []int{
		model.LogTypeConsume,
		model.LogTypeTopup,
		model.LogTypeRefund,
		model.LogTypeSystem,
		model.LogTypeManage,
	}
	if err := model.LOG_DB.Model(&model.Log{}).
		Select("id", "user_id", "created_at", "type", "username", "token_name", "model_name", "quota", "prompt_tokens", "completion_tokens", "use_time", "is_stream", "channel_id", "group").
		Where("created_at >= ? AND created_at <= ? AND type IN ?", start, end, usageTypes).
		Order("created_at ASC, id ASC").Limit(financeExportMaxRows).Find(&bundle.Usage).Error; err != nil {
		return bundle, err
	}
	if err := model.DB.Model(&model.OpenSourceBountyLedger{}).
		Select("id", "project_id", "challenge_id", "user_id", "counterparty_user_id", "kind", "quota", "note", "created_at").
		Where("created_at >= ? AND created_at <= ?", start, end).
		Order("created_at ASC, id ASC").Limit(financeExportMaxRows).Find(&bundle.BountyLedger).Error; err != nil {
		return bundle, err
	}
	if err := model.DB.Model(&model.Checkin{}).
		Select("id", "user_id", "checkin_date", "quota_awarded", "created_at").
		Where("created_at >= ? AND created_at <= ?", start, end).
		Order("created_at ASC, id ASC").Limit(financeExportMaxRows).Find(&bundle.Checkins).Error; err != nil {
		return bundle, err
	}
	if err := model.DB.Model(&model.Redemption{}).
		Select("id", "user_id", "status", "name", "quota", "created_time", "redeemed_time", "used_user_id", "expired_time").
		Where("deleted_at IS NULL").
		Order("id ASC").Limit(financeExportMaxRows).Find(&bundle.Redemptions).Error; err != nil {
		return bundle, err
	}
	if err := model.DB.Model(&model.UserSubscription{}).
		Select("id", "user_id", "plan_id", "amount_total", "amount_used", "start_time", "end_time", "status", "source", "last_reset_time", "next_reset_time", "upgrade_group", "prev_user_group", "downgrade_group", "allow_wallet_overflow", "created_at", "updated_at").
		Order("id ASC").Limit(financeExportMaxRows).Find(&bundle.UserSubscriptions).Error; err != nil {
		return bundle, err
	}

	bundle.Manifest.Rows["options"] = len(bundle.Options)
	bundle.Manifest.Rows["effective_model_pricing"] = len(bundle.EffectivePricing)
	bundle.Manifest.Rows["users"] = len(bundle.Users)
	bundle.Manifest.Rows["channels"] = len(bundle.Channels)
	bundle.Manifest.Rows["plans"] = len(bundle.Plans)
	bundle.Manifest.Rows["topups"] = len(bundle.TopUps)
	bundle.Manifest.Rows["subscription_orders"] = len(bundle.SubscriptionOrders)
	bundle.Manifest.Rows["usage"] = len(bundle.Usage)
	bundle.Manifest.Rows["bounty_ledger"] = len(bundle.BountyLedger)
	bundle.Manifest.Rows["checkins"] = len(bundle.Checkins)
	bundle.Manifest.Rows["redemptions"] = len(bundle.Redemptions)
	bundle.Manifest.Rows["user_subscriptions"] = len(bundle.UserSubscriptions)
	bundle.Manifest.Truncated["topups"] = len(bundle.TopUps) == financeExportMaxRows
	bundle.Manifest.Truncated["subscription_orders"] = len(bundle.SubscriptionOrders) == financeExportMaxRows
	bundle.Manifest.Truncated["usage"] = len(bundle.Usage) == financeExportMaxRows
	bundle.Manifest.Truncated["bounty_ledger"] = len(bundle.BountyLedger) == financeExportMaxRows
	bundle.Manifest.Truncated["checkins"] = len(bundle.Checkins) == financeExportMaxRows
	bundle.Manifest.Truncated["redemptions"] = len(bundle.Redemptions) == financeExportMaxRows
	bundle.Manifest.Truncated["user_subscriptions"] = len(bundle.UserSubscriptions) == financeExportMaxRows
	return bundle, nil
}

func jsonFinanceFile(value any) ([]byte, error) {
	return json.MarshalIndent(value, "", "  ")
}

func decodeFinanceOption(value string) any {
	var decoded any
	if err := json.Unmarshal([]byte(value), &decoded); err == nil {
		return decoded
	}
	return value
}

func decodeFinanceFloatMap(value string) map[string]float64 {
	if strings.TrimSpace(value) == "" {
		return nil
	}
	var decoded map[string]float64
	if err := json.Unmarshal([]byte(value), &decoded); err != nil {
		return nil
	}
	return decoded
}

func sanitizeFinanceBaseURL(value *string) *string {
	if value == nil {
		return nil
	}
	parsed, err := url.Parse(strings.TrimSpace(*value))
	if err != nil || parsed.Scheme == "" || parsed.Host == "" {
		return nil
	}
	sanitized := parsed.Scheme + "://" + parsed.Host
	return &sanitized
}

func applyFinanceUserRatios(users []financeUserExport, options map[string]string) {
	groupRatios := decodeFinanceFloatMap(options["GroupRatio"])
	topupGroupRatios := decodeFinanceFloatMap(options["TopupGroupRatio"])
	for index := range users {
		if ratio, ok := groupRatios[users[index].Group]; ok {
			value := ratio
			users[index].GroupRatio = &value
		}
		if ratio, ok := topupGroupRatios[users[index].Group]; ok {
			value := ratio
			users[index].TopupGroupRatio = &value
		}
	}
}

func financeExportFiles(bundle financeExportBundle) (map[string][]byte, error) {
	manifest, err := jsonFinanceFile(bundle.Manifest)
	if err != nil {
		return nil, err
	}
	options, err := jsonFinanceFile(bundle.Options)
	if err != nil {
		return nil, err
	}
	modelPrices, err := jsonFinanceFile(map[string]any{
		"model_prices":            decodeFinanceOption(bundle.Options["ModelPrice"]),
		"model_ratios":            decodeFinanceOption(bundle.Options["ModelRatio"]),
		"completion_ratios":       decodeFinanceOption(bundle.Options["CompletionRatio"]),
		"cache_ratios":            decodeFinanceOption(bundle.Options["CacheRatio"]),
		"create_cache_ratios":     decodeFinanceOption(bundle.Options["CreateCacheRatio"]),
		"image_ratios":            decodeFinanceOption(bundle.Options["ImageRatio"]),
		"audio_ratios":            decodeFinanceOption(bundle.Options["AudioRatio"]),
		"audio_completion_ratios": decodeFinanceOption(bundle.Options["AudioCompletionRatio"]),
		"tool_prices":             decodeFinanceOption(bundle.Options["tool_price_setting.prices"]),
		"billing_modes":           decodeFinanceOption(bundle.Options["billing_setting.billing_mode"]),
		"billing_expressions":     decodeFinanceOption(bundle.Options["billing_setting.billing_expr"]),
	})
	if err != nil {
		return nil, err
	}
	effectivePricing, err := jsonFinanceFile(bundle.EffectivePricing)
	if err != nil {
		return nil, err
	}
	users, err := jsonFinanceFile(bundle.Users)
	if err != nil {
		return nil, err
	}
	channels, err := jsonFinanceFile(bundle.Channels)
	if err != nil {
		return nil, err
	}
	plans, err := jsonFinanceFile(bundle.Plans)
	if err != nil {
		return nil, err
	}
	topups, err := jsonFinanceFile(bundle.TopUps)
	if err != nil {
		return nil, err
	}
	orders, err := jsonFinanceFile(bundle.SubscriptionOrders)
	if err != nil {
		return nil, err
	}
	usage, err := jsonFinanceFile(bundle.Usage)
	if err != nil {
		return nil, err
	}
	ledger, err := jsonFinanceFile(bundle.BountyLedger)
	if err != nil {
		return nil, err
	}
	checkins, err := jsonFinanceFile(bundle.Checkins)
	if err != nil {
		return nil, err
	}
	redemptions, err := jsonFinanceFile(bundle.Redemptions)
	if err != nil {
		return nil, err
	}
	userSubscriptions, err := jsonFinanceFile(bundle.UserSubscriptions)
	if err != nil {
		return nil, err
	}
	return map[string][]byte{
		"manifest.json":                manifest,
		"financial-options.json":       options,
		"model-prices-and-ratios.json": modelPrices,
		"effective-model-pricing.json": effectivePricing,
		"users-balances.json":          users,
		"channels-pricing.json":        channels,
		"subscription-plans.json":      plans,
		"topups.json":                  topups,
		"subscription-orders.json":     orders,
		"usage-billing-records.json":   usage,
		"bounty-ledger.json":           ledger,
		"checkins.json":                checkins,
		"redemptions.json":             redemptions,
		"user-subscriptions.json":      userSubscriptions,
	}, nil
}

func financeExportText(files map[string][]byte) []byte {
	var buffer bytes.Buffer
	buffer.WriteString("LMM Finance Analysis Export\n")
	buffer.WriteString("========================================\n\n")
	for _, name := range []string{
		"manifest.json",
		"financial-options.json",
		"model-prices-and-ratios.json",
		"effective-model-pricing.json",
		"users-balances.json",
		"channels-pricing.json",
		"subscription-plans.json",
		"topups.json",
		"subscription-orders.json",
		"usage-billing-records.json",
		"bounty-ledger.json",
		"checkins.json",
		"redemptions.json",
		"user-subscriptions.json",
	} {
		buffer.WriteString("## ")
		buffer.WriteString(name)
		buffer.WriteString("\n")
		buffer.Write(files[name])
		buffer.WriteString("\n\n")
	}
	return buffer.Bytes()
}

func writeFinanceZip(c *gin.Context, files map[string][]byte) error {
	filename := fmt.Sprintf("lmm-finance-export-%s.zip", time.Now().UTC().Format("20060102-150405"))
	c.Header("Content-Type", "application/zip")
	c.Header("Content-Disposition", fmt.Sprintf(`attachment; filename="%s"`, filename))
	c.Header("Cache-Control", "no-store")
	c.Header("X-Content-Type-Options", "nosniff")
	writer := zip.NewWriter(c.Writer)
	for _, name := range []string{
		"manifest.json",
		"financial-options.json",
		"model-prices-and-ratios.json",
		"effective-model-pricing.json",
		"users-balances.json",
		"channels-pricing.json",
		"subscription-plans.json",
		"topups.json",
		"subscription-orders.json",
		"usage-billing-records.json",
		"bounty-ledger.json",
		"checkins.json",
		"redemptions.json",
		"user-subscriptions.json",
	} {
		entry, err := writer.Create(name)
		if err != nil {
			_ = writer.Close()
			return err
		}
		if _, err := entry.Write(files[name]); err != nil {
			_ = writer.Close()
			return err
		}
	}
	return writer.Close()
}

// ExportFinancialData creates a redacted, admin-only analysis snapshot. The
// response is either a ZIP (default) or a plain-text bundle for clipboard use.
func ExportFinancialData(c *gin.Context) {
	format := strings.ToLower(strings.TrimSpace(c.DefaultQuery("format", "zip")))
	if format != "zip" && format != "text" {
		common.ApiErrorMsg(c, "format must be zip or text")
		return
	}
	start, end, err := parseFinanceExportWindow(c)
	if err != nil {
		common.ApiErrorMsg(c, err.Error())
		return
	}
	bundle, err := loadFinanceExportBundle(start, end)
	if err != nil {
		common.ApiError(c, err)
		return
	}
	files, err := financeExportFiles(bundle)
	if err != nil {
		common.ApiError(c, err)
		return
	}
	model.RecordOperationAuditLog(
		c.GetInt("id"),
		"Financial analysis export generated",
		c.ClientIP(),
		"finance.export",
		map[string]interface{}{
			"format":          format,
			"start_timestamp": start,
			"end_timestamp":   end,
			"rows":            bundle.Manifest.Rows,
		},
		nil,
		nil,
	)
	if format == "text" {
		c.Data(http.StatusOK, "text/plain; charset=utf-8", financeExportText(files))
		return
	}
	if err := writeFinanceZip(c, files); err != nil {
		// Headers may already be committed; only log the failure instead of
		// attempting to write a second JSON response.
		return
	}
}
