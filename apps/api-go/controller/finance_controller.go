package controller

import (
	"errors"
	"fmt"
	"math"
	"net/http"
	"sort"
	"strconv"
	"strings"
	"time"

	"github.com/QuantumNous/new-api/common"
	"github.com/QuantumNous/new-api/model"
	"github.com/gin-gonic/gin"
	"gorm.io/gorm"
)

const (
	financeDashboardDefaultWindow = 30 * 24 * 60 * 60
	financeDashboardMaxWindow     = 366 * 24 * 60 * 60
	financeDashboardMaxSourceRows = 100_000
	// Finance dashboards are read-only, but a busy installation can still
	// have hundreds of thousands of source rows in one window. Keep each
	// database round-trip small so the accumulator never retains raw source
	// rows after a batch has been processed.
	financeDashboardBatchSize  = 1_000
	financeDashboardMaxEntries = 100
)

type financeRange struct {
	Start int64 `json:"start"`
	End   int64 `json:"end"`
}

type financeMethodMetric struct {
	Method       string `json:"method"`
	Provider     string `json:"provider"`
	Category     string `json:"category,omitempty"`
	AmountMicros int64  `json:"amount_micros"`
	Orders       int64  `json:"orders"`
	Users        int64  `json:"users"`
	TokenUnits   int64  `json:"token_units"`
}

type financeDailyMetric struct {
	Date          string `json:"date"`
	RevenueMicros int64  `json:"revenue_micros"`
	ExpenseMicros int64  `json:"expense_micros"`
	ProfitMicros  int64  `json:"profit_micros"`
	TokenUnits    int64  `json:"token_units"`
	Requests      int64  `json:"requests"`
}

type financeUserMetric struct {
	UserID          int   `json:"user_id"`
	RevenueMicros   int64 `json:"revenue_micros"`
	ExpenseMicros   int64 `json:"expense_micros"`
	TokenCostMicros int64 `json:"token_cost_micros"`
	TokenUnits      int64 `json:"token_units"`
	Requests        int64 `json:"requests"`
}

type financeTokenMetric struct {
	PromptTokens        int64 `json:"prompt_tokens"`
	CompletionTokens    int64 `json:"completion_tokens"`
	TotalTokens         int64 `json:"total_tokens"`
	Requests            int64 `json:"requests"`
	EstimatedCostMicros int64 `json:"estimated_cost_micros"`
	UnpricedRequests    int64 `json:"unpriced_requests"`
}

type financeOverview struct {
	Range           financeRange                 `json:"range"`
	Currency        string                       `json:"currency"`
	RevenueMicros   int64                        `json:"revenue_micros"`
	ExpenseMicros   int64                        `json:"expense_micros"`
	ProfitMicros    int64                        `json:"profit_micros"`
	RevenueByMethod []financeMethodMetric        `json:"revenue_by_method"`
	ExpenseByMethod []financeMethodMetric        `json:"expense_by_method"`
	Tokens          financeTokenMetric           `json:"tokens"`
	Daily           []financeDailyMetric         `json:"daily"`
	Users           []financeUserMetric          `json:"users"`
	PaymentMethods  []model.FinancePaymentMethod `json:"payment_methods"`
	SourcesBounded  bool                         `json:"sources_bounded"`
}

type financeAccumulator struct {
	overview    financeOverview
	methods     map[string]*financeMethodMetric
	expenses    map[string]*financeMethodMetric
	daily       map[string]*financeDailyMetric
	users       map[int]*financeUserMetric
	methodUsers map[string]map[int]struct{}
}

func newFinanceAccumulator(start, end int64, paymentMethods []model.FinancePaymentMethod) *financeAccumulator {
	return &financeAccumulator{
		overview: financeOverview{Range: financeRange{Start: start, End: end}, Currency: model.FinanceCurrencyUSD, PaymentMethods: paymentMethods, SourcesBounded: true},
		methods:  make(map[string]*financeMethodMetric), expenses: make(map[string]*financeMethodMetric), daily: make(map[string]*financeDailyMetric), users: make(map[int]*financeUserMetric), methodUsers: make(map[string]map[int]struct{}),
	}
}

func (a *financeAccumulator) dailyMetric(timestamp int64) *financeDailyMetric {
	key := time.Unix(timestamp, 0).UTC().Format("2006-01-02")
	metric := a.daily[key]
	if metric == nil {
		metric = &financeDailyMetric{Date: key}
		a.daily[key] = metric
	}
	return metric
}

func (a *financeAccumulator) userMetric(userID int) *financeUserMetric {
	if userID <= 0 {
		return nil
	}
	metric := a.users[userID]
	if metric == nil {
		metric = &financeUserMetric{UserID: userID}
		a.users[userID] = metric
	}
	return metric
}

func (a *financeAccumulator) addRevenue(method, provider string, amount, timestamp int64, userID int) {
	if amount <= 0 {
		return
	}
	key := strings.TrimSpace(method) + "\x00" + strings.TrimSpace(provider)
	metric := a.methods[key]
	if metric == nil {
		metric = &financeMethodMetric{Method: strings.TrimSpace(method), Provider: strings.TrimSpace(provider)}
		a.methods[key] = metric
	}
	metric.AmountMicros += amount
	metric.Orders++
	if userID > 0 {
		if a.methodUsers[key] == nil {
			a.methodUsers[key] = make(map[int]struct{})
		}
		a.methodUsers[key][userID] = struct{}{}
		if user := a.userMetric(userID); user != nil {
			user.RevenueMicros += amount
		}
	}
	a.overview.RevenueMicros += amount
	a.dailyMetric(timestamp).RevenueMicros += amount
}

func (a *financeAccumulator) addExpense(category, method, provider string, amount, timestamp int64, userID int) {
	if amount <= 0 {
		return
	}
	key := strings.TrimSpace(category) + "\x00" + strings.TrimSpace(method) + "\x00" + strings.TrimSpace(provider)
	metric := a.expenses[key]
	if metric == nil {
		metric = &financeMethodMetric{Method: strings.TrimSpace(method), Provider: strings.TrimSpace(provider)}
		a.expenses[key] = metric
	}
	metric.Category = strings.TrimSpace(category)
	metric.AmountMicros += amount
	if userID > 0 {
		if user := a.userMetric(userID); user != nil {
			user.ExpenseMicros += amount
		}
	}
	a.overview.ExpenseMicros += amount
	a.dailyMetric(timestamp).ExpenseMicros += amount
}

func (a *financeAccumulator) addUsage(userID int, timestamp int64, prompt, completion int, estimatedCost int64, priced bool) {
	if prompt < 0 {
		prompt = 0
	}
	if completion < 0 {
		completion = 0
	}
	total := int64(prompt + completion)
	a.overview.Tokens.PromptTokens += int64(prompt)
	a.overview.Tokens.CompletionTokens += int64(completion)
	a.overview.Tokens.TotalTokens += total
	a.overview.Tokens.Requests++
	a.overview.Tokens.EstimatedCostMicros += estimatedCost
	if !priced {
		a.overview.Tokens.UnpricedRequests++
	}
	daily := a.dailyMetric(timestamp)
	daily.TokenUnits += total
	daily.Requests++
	if user := a.userMetric(userID); user != nil {
		user.TokenUnits += total
		user.Requests++
		user.TokenCostMicros += estimatedCost
	}
	if estimatedCost > 0 {
		a.addExpense("token_cost", "", "", estimatedCost, timestamp, userID)
	}
}

func (a *financeAccumulator) finish() financeOverview {
	for key, metric := range a.methods {
		metric.Users = int64(len(a.methodUsers[key]))
		a.overview.RevenueByMethod = append(a.overview.RevenueByMethod, *metric)
	}
	for _, metric := range a.expenses {
		a.overview.ExpenseByMethod = append(a.overview.ExpenseByMethod, *metric)
	}
	for _, metric := range a.daily {
		metric.ProfitMicros = metric.RevenueMicros - metric.ExpenseMicros
		a.overview.Daily = append(a.overview.Daily, *metric)
	}
	for _, metric := range a.users {
		a.overview.Users = append(a.overview.Users, *metric)
	}
	sort.Slice(a.overview.RevenueByMethod, func(i, j int) bool {
		return a.overview.RevenueByMethod[i].AmountMicros > a.overview.RevenueByMethod[j].AmountMicros
	})
	sort.Slice(a.overview.ExpenseByMethod, func(i, j int) bool {
		return a.overview.ExpenseByMethod[i].AmountMicros > a.overview.ExpenseByMethod[j].AmountMicros
	})
	sort.Slice(a.overview.Daily, func(i, j int) bool { return a.overview.Daily[i].Date < a.overview.Daily[j].Date })
	sort.Slice(a.overview.Users, func(i, j int) bool {
		return a.overview.Users[i].ExpenseMicros+a.overview.Users[i].RevenueMicros > a.overview.Users[j].ExpenseMicros+a.overview.Users[j].RevenueMicros
	})
	if len(a.overview.Users) > 100 {
		a.overview.Users = a.overview.Users[:100]
	}
	a.overview.ProfitMicros = a.overview.RevenueMicros - a.overview.ExpenseMicros
	return a.overview
}

func parseFinanceDashboardRange(c *gin.Context) (int64, int64, error) {
	now := time.Now().Unix()
	start, end := now-financeDashboardDefaultWindow, now
	for name, target := range map[string]*int64{"start_timestamp": &start, "end_timestamp": &end} {
		value := strings.TrimSpace(c.Query(name))
		if value == "" {
			continue
		}
		parsed, err := strconv.ParseInt(value, 10, 64)
		if err != nil || parsed <= 0 {
			return 0, 0, fmt.Errorf("invalid %s", name)
		}
		*target = parsed
	}
	if start >= end || end-start > financeDashboardMaxWindow {
		return 0, 0, errors.New("invalid finance dashboard range")
	}
	return start, end, nil
}

func financeMethodFromTopUp(topUp model.TopUp) (string, string) {
	method, provider := strings.TrimSpace(topUp.PaymentMethod), strings.TrimSpace(topUp.PaymentProvider)
	if method == "" {
		method = provider
	}
	if provider == "" {
		provider = method
	}
	return method, provider
}

func financeMicrosFromFloat(value float64) int64 {
	if value <= 0 || math.IsNaN(value) || math.IsInf(value, 0) {
		return 0
	}
	return int64(math.Round(value * 1_000_000))
}

func financeTopUpAmount(topUp model.TopUp) int64 {
	if topUp.SettledAmountMicros > 0 {
		return topUp.SettledAmountMicros
	}
	if topUp.ExpectedAmountMicros > 0 {
		return topUp.ExpectedAmountMicros
	}
	return financeMicrosFromFloat(topUp.Money)
}

func financePaymentMethodAllowed(method string, configs map[string]model.FinancePaymentMethod) bool {
	if method == model.PaymentMethodBalance || method == model.PaymentProviderBalance {
		return false
	}
	config, ok := configs[method]
	return !ok || (config.Enabled && config.IncludeRevenue)
}

func loadFinancePaymentMethods() ([]model.FinancePaymentMethod, map[string]model.FinancePaymentMethod, error) {
	var configs []model.FinancePaymentMethod
	if err := model.DB.Order("method asc").Find(&configs).Error; err != nil {
		return nil, nil, err
	}
	seen := make(map[string]bool, len(configs))
	byMethod := make(map[string]model.FinancePaymentMethod, len(configs))
	for _, config := range configs {
		seen[config.Method] = true
		byMethod[config.Method] = config
	}
	known := []string{model.PaymentProviderStripe, model.PaymentProviderCreem, model.PaymentProviderEpay, model.PaymentProviderFastPay, model.PaymentProviderWaffo, model.PaymentProviderWaffoPancake}
	var observed []string
	model.DB.Model(&model.TopUp{}).Where("payment_method <> '' OR payment_provider <> ''").Distinct().Pluck("payment_method", &observed)
	known = append(known, observed...)
	for _, method := range known {
		method = strings.TrimSpace(method)
		if method == "" || seen[method] || method == model.PaymentProviderBalance {
			continue
		}
		config := model.FinancePaymentMethod{Method: method, Label: method, Enabled: true, IncludeRevenue: true}
		configs = append(configs, config)
		byMethod[method] = config
		seen[method] = true
	}
	sort.Slice(configs, func(i, j int) bool { return configs[i].Method < configs[j].Method })
	return configs, byMethod, nil
}

func financeBatchLimit(processed int) int {
	remaining := financeDashboardMaxSourceRows - processed
	if remaining <= 0 {
		return 0
	}
	if remaining < financeDashboardBatchSize {
		return remaining
	}
	return financeDashboardBatchSize
}

// iterateFinanceSource reads a source ordered by its timestamp and primary
// key. Offset pagination becomes increasingly expensive for large windows and
// can repeat/skip rows when a new payment or log arrives while the dashboard
// is loading. The composite cursor keeps every batch bounded and stable.
func iterateFinanceSource[T any](base *gorm.DB, timestampColumn string, visit func(T) error) error {
	if base == nil || visit == nil {
		return gorm.ErrInvalidData
	}
	processed := 0
	var lastTimestamp int64
	var lastID int64
	for {
		limit := financeBatchLimit(processed)
		if limit == 0 {
			return nil
		}
		rows := make([]T, 0, limit)
		query := base.Session(&gorm.Session{}).
			Where("("+timestampColumn+" > ? OR ("+timestampColumn+" = ? AND id > ?))", lastTimestamp, lastTimestamp, lastID).
			Order(timestampColumn + " ASC, id ASC").
			Limit(limit)
		if err := query.Find(&rows).Error; err != nil {
			return err
		}
		if len(rows) == 0 {
			return nil
		}
		for index := range rows {
			if err := visit(rows[index]); err != nil {
				return err
			}
		}
		processed += len(rows)
		lastTimestamp, lastID = financeSourceCursor(rows[len(rows)-1], timestampColumn)
		if len(rows) < limit {
			return nil
		}
	}
}

// financeSourceCursor extracts the two cursor fields without retaining a
// second copy of any source row. The concrete source types are intentionally
// kept here so the query helper remains generic while its public JSON shape
// stays unchanged.
func financeSourceCursor[T any](row T, timestampColumn string) (int64, int64) {
	switch value := any(row).(type) {
	case model.TopUp:
		if timestampColumn == "complete_time" {
			return value.CompleteTime, int64(value.Id)
		}
	case model.SubscriptionOrder:
		if timestampColumn == "complete_time" {
			return value.CompleteTime, int64(value.Id)
		}
	case model.Log:
		if timestampColumn == "created_at" {
			return value.CreatedAt, int64(value.Id)
		}
	case model.FinanceLedgerEntry:
		if timestampColumn == "occurred_at" {
			return value.OccurredAt, value.Id
		}
	}
	return 0, 0
}

func buildFinanceOverview(start, end int64, userFilter int, methodFilter string) (financeOverview, error) {
	methods, configMap, err := loadFinancePaymentMethods()
	if err != nil {
		return financeOverview{}, err
	}
	a := newFinanceAccumulator(start, end, methods)
	tx := model.DB.Where("status = ? AND complete_time >= ? AND complete_time < ?", common.TopUpStatusSuccess, start, end)
	if userFilter > 0 {
		tx = tx.Where("user_id = ?", userFilter)
	}
	if err := iterateFinanceSource[model.TopUp](tx.Select("id, user_id, expected_amount_micros, settled_amount_micros, money, payment_method, payment_provider, create_time, complete_time, status"), "complete_time", func(topUp model.TopUp) error {
		method, provider := financeMethodFromTopUp(topUp)
		if methodFilter != "" && method != methodFilter {
			return nil
		}
		if !financePaymentMethodAllowed(method, configMap) {
			return nil
		}
		timestamp := topUp.CompleteTime
		if timestamp <= 0 {
			timestamp = topUp.CreateTime
		}
		a.addRevenue(method, provider, financeTopUpAmount(topUp), timestamp, topUp.UserId)
		return nil
	}); err != nil {
		return financeOverview{}, err
	}
	tx = model.DB.Where("status = ? AND complete_time >= ? AND complete_time < ?", common.TopUpStatusSuccess, start, end)
	if userFilter > 0 {
		tx = tx.Where("user_id = ?", userFilter)
	}
	if err := iterateFinanceSource[model.SubscriptionOrder](tx.Select("id, user_id, money, payment_method, payment_provider, create_time, complete_time, status"), "complete_time", func(order model.SubscriptionOrder) error {
		method, provider := strings.TrimSpace(order.PaymentMethod), strings.TrimSpace(order.PaymentProvider)
		if method == "" {
			method = provider
		}
		if provider == "" {
			provider = method
		}
		if methodFilter != "" && method != methodFilter {
			return nil
		}
		if !financePaymentMethodAllowed(method, configMap) {
			return nil
		}
		timestamp := order.CompleteTime
		if timestamp <= 0 {
			timestamp = order.CreateTime
		}
		a.addRevenue(method, provider, financeMicrosFromFloat(order.Money), timestamp, order.UserId)
		return nil
	}); err != nil {
		return financeOverview{}, err
	}
	tx = model.LOG_DB.Where("type = ? AND created_at >= ? AND created_at < ?", model.LogTypeConsume, start, end)
	if userFilter > 0 {
		tx = tx.Where("user_id = ?", userFilter)
	}
	if err := iterateFinanceSource[model.Log](tx.Select("id, user_id, created_at, type, prompt_tokens, completion_tokens, other"), "created_at", func(log model.Log) error {
		other, _ := common.StrToMap(log.Other)
		price, priced := 0.0, false
		if raw, ok := other["model_price"].(float64); ok && raw > 0 {
			price, priced = raw, true
		}
		cost := int64(math.Round(price * float64(max(0, log.PromptTokens+log.CompletionTokens))))
		a.addUsage(log.UserId, log.CreatedAt, log.PromptTokens, log.CompletionTokens, cost, priced)
		return nil
	}); err != nil {
		return financeOverview{}, err
	}
	tx = model.DB.Where("occurred_at >= ? AND occurred_at < ?", start, end)
	if userFilter > 0 {
		tx = tx.Where("user_id = ?", userFilter)
	}
	if methodFilter != "" {
		tx = tx.Where("payment_method = ?", methodFilter)
	}
	if err := iterateFinanceSource[model.FinanceLedgerEntry](tx.Select("id, entry_type, category, amount_micros, currency, direction, payment_method, payment_provider, user_id, source_type, source_id, token_units, occurred_at, created_at, created_by, reversal_of_id"), "occurred_at", func(entry model.FinanceLedgerEntry) error {
		if entry.EntryType == model.FinanceEntryRevenue {
			if entry.Direction == model.FinanceDirectionCredit {
				a.addRevenue(entry.PaymentMethod, entry.PaymentProvider, entry.AmountMicros, entry.OccurredAt, derefFinanceUser(entry.UserId))
			} else {
				a.addExpense("revenue_reversal", entry.PaymentMethod, entry.PaymentProvider, entry.AmountMicros, entry.OccurredAt, derefFinanceUser(entry.UserId))
			}
		} else if entry.EntryType == model.FinanceEntryExpense || entry.EntryType == model.FinanceEntryTokenCost {
			a.addExpense(entry.Category, entry.PaymentMethod, entry.PaymentProvider, entry.AmountMicros, entry.OccurredAt, derefFinanceUser(entry.UserId))
		}
		return nil
	}); err != nil {
		return financeOverview{}, err
	}
	return a.finish(), nil
}

func derefFinanceUser(value *int) int {
	if value == nil {
		return 0
	}
	return *value
}

func financeOverviewHandler(c *gin.Context) {
	start, end, err := parseFinanceDashboardRange(c)
	if err != nil {
		c.AbortWithStatusJSON(http.StatusBadRequest, gin.H{"success": false, "message": err.Error()})
		return
	}
	userID, _ := strconv.Atoi(strings.TrimSpace(c.Query("user_id")))
	method := strings.TrimSpace(c.Query("payment_method"))
	view, err := buildFinanceOverview(start, end, userID, method)
	if err != nil {
		common.ApiError(c, err)
		return
	}
	common.ApiSuccess(c, view)
}

func financeUsersHandler(c *gin.Context) {
	start, end, err := parseFinanceDashboardRange(c)
	if err != nil {
		c.AbortWithStatusJSON(http.StatusBadRequest, gin.H{"success": false, "message": err.Error()})
		return
	}
	view, err := buildFinanceOverview(start, end, 0, "")
	if err != nil {
		common.ApiError(c, err)
		return
	}
	common.ApiSuccess(c, gin.H{"range": view.Range, "users": view.Users})
}

func financeUserHandler(c *gin.Context) {
	userID, err := strconv.Atoi(c.Param("user_id"))
	if err != nil || userID <= 0 {
		c.AbortWithStatusJSON(http.StatusBadRequest, gin.H{"success": false, "message": "invalid user_id"})
		return
	}
	start, end, err := parseFinanceDashboardRange(c)
	if err != nil {
		c.AbortWithStatusJSON(http.StatusBadRequest, gin.H{"success": false, "message": err.Error()})
		return
	}
	view, err := buildFinanceOverview(start, end, userID, strings.TrimSpace(c.Query("payment_method")))
	if err != nil {
		common.ApiError(c, err)
		return
	}
	common.ApiSuccess(c, view)
}

type financeEntryInput struct {
	EntryType       string `json:"entry_type"`
	Category        string `json:"category"`
	AmountMicros    int64  `json:"amount_micros"`
	Currency        string `json:"currency"`
	PaymentMethod   string `json:"payment_method"`
	PaymentProvider string `json:"payment_provider"`
	UserID          *int   `json:"user_id"`
	Note            string `json:"note"`
	OccurredAt      int64  `json:"occurred_at"`
	IdempotencyKey  string `json:"idempotency_key"`
}

func createFinanceEntryHandler(c *gin.Context) {
	var input financeEntryInput
	if err := c.ShouldBindJSON(&input); err != nil {
		c.AbortWithStatusJSON(http.StatusBadRequest, gin.H{"success": false, "message": "invalid finance entry"})
		return
	}
	if strings.TrimSpace(input.EntryType) != model.FinanceEntryExpense {
		c.AbortWithStatusJSON(http.StatusUnprocessableEntity, gin.H{"success": false, "message": "manual entries must be expenses"})
		return
	}
	if input.OccurredAt == 0 {
		input.OccurredAt = time.Now().Unix()
	}
	entry, err := model.AppendFinanceLedgerEntry(&model.FinanceLedgerEntry{EntryType: model.FinanceEntryExpense, Category: input.Category, AmountMicros: input.AmountMicros, Currency: input.Currency, Direction: model.FinanceDirectionDebit, PaymentMethod: input.PaymentMethod, PaymentProvider: input.PaymentProvider, UserId: input.UserID, SourceType: model.FinanceSourceManual, Note: input.Note, OccurredAt: input.OccurredAt, CreatedBy: c.GetInt("id"), IdempotencyKey: input.IdempotencyKey})
	if err != nil {
		c.AbortWithStatusJSON(http.StatusUnprocessableEntity, gin.H{"success": false, "message": err.Error()})
		return
	}
	common.ApiSuccess(c, gin.H{"entry": entry})
}

func reverseFinanceEntryHandler(c *gin.Context) {
	id, err := strconv.ParseInt(c.Param("entry_id"), 10, 64)
	if err != nil || id <= 0 {
		c.AbortWithStatusJSON(http.StatusBadRequest, gin.H{"success": false, "message": "invalid entry_id"})
		return
	}
	entry, err := model.ReverseFinanceLedgerEntry(id, c.GetInt("id"), time.Now().Unix())
	if err != nil {
		status := http.StatusUnprocessableEntity
		if errors.Is(err, model.ErrFinanceEntryNotFound) {
			status = http.StatusNotFound
		}
		c.AbortWithStatusJSON(status, gin.H{"success": false, "message": err.Error()})
		return
	}
	common.ApiSuccess(c, gin.H{"entry": entry})
}

func financeEntriesHandler(c *gin.Context) {
	limit, _ := strconv.Atoi(c.DefaultQuery("limit", "50"))
	if limit < 1 {
		limit = 1
	}
	if limit > financeDashboardMaxEntries {
		limit = financeDashboardMaxEntries
	}
	query := model.DB.Order("occurred_at desc, id desc").Limit(limit)
	if value := strings.TrimSpace(c.Query("entry_type")); value != "" {
		query = query.Where("entry_type = ?", value)
	}
	var entries []model.FinanceLedgerEntry
	if err := query.Find(&entries).Error; err != nil {
		common.ApiError(c, err)
		return
	}
	common.ApiSuccess(c, gin.H{"entries": entries})
}

type financePaymentMethodInput struct {
	Label          *string `json:"label"`
	Enabled        *bool   `json:"enabled"`
	IncludeRevenue *bool   `json:"include_revenue"`
}

func updateFinancePaymentMethodHandler(c *gin.Context) {
	method := strings.TrimSpace(c.Param("method"))
	if method == "" || len(method) > 64 {
		c.AbortWithStatusJSON(http.StatusBadRequest, gin.H{"success": false, "message": "invalid payment method"})
		return
	}
	var input financePaymentMethodInput
	if err := c.ShouldBindJSON(&input); err != nil {
		c.AbortWithStatusJSON(http.StatusBadRequest, gin.H{"success": false, "message": "invalid payment method settings"})
		return
	}
	var config model.FinancePaymentMethod
	err := model.DB.Where("method = ?", method).First(&config).Error
	if errors.Is(err, gorm.ErrRecordNotFound) {
		config = model.FinancePaymentMethod{Method: method, Label: method, Enabled: true, IncludeRevenue: true, CreatedAt: time.Now().Unix(), UpdatedAt: time.Now().Unix(), CreatedBy: c.GetInt("id")}
		err = nil
	}
	if err != nil {
		common.ApiError(c, err)
		return
	}
	if input.Label != nil {
		config.Label = strings.TrimSpace(*input.Label)
	}
	if input.Enabled != nil {
		config.Enabled = *input.Enabled
	}
	if input.IncludeRevenue != nil {
		config.IncludeRevenue = *input.IncludeRevenue
	}
	if config.Label == "" {
		config.Label = method
	}
	config.UpdatedAt = time.Now().Unix()
	config.CreatedBy = c.GetInt("id")
	if err := model.DB.Save(&config).Error; err != nil {
		common.ApiError(c, err)
		return
	}
	common.ApiSuccess(c, config)
}

func listFinancePaymentMethodsHandler(c *gin.Context) {
	methods, _, err := loadFinancePaymentMethods()
	if err != nil {
		common.ApiError(c, err)
		return
	}
	common.ApiSuccess(c, gin.H{"methods": methods})
}

func GetFinanceOverview(c *gin.Context)         { financeOverviewHandler(c) }
func GetFinanceUsers(c *gin.Context)            { financeUsersHandler(c) }
func GetFinanceUser(c *gin.Context)             { financeUserHandler(c) }
func ListFinanceEntries(c *gin.Context)         { financeEntriesHandler(c) }
func CreateFinanceEntry(c *gin.Context)         { createFinanceEntryHandler(c) }
func ReverseFinanceEntry(c *gin.Context)        { reverseFinanceEntryHandler(c) }
func ListFinancePaymentMethods(c *gin.Context)  { listFinancePaymentMethodsHandler(c) }
func UpdateFinancePaymentMethod(c *gin.Context) { updateFinancePaymentMethodHandler(c) }
