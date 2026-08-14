package model

import (
	"context"
	"errors"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"gorm.io/gorm"
)

const (
	AdvancedSecurityDecisionBlocked = "blocked"
	AdvancedSecurityDecisionAudited = "audited"
)

// AdvancedSecurityEvent is a structured audit row for one matched rule. It
// intentionally stores digests instead of prompt text or matcher patterns.
// The table lives in the primary database so it remains available even when
// LOG_SQL_DSN points at a ClickHouse/isolated log database.
type AdvancedSecurityEvent struct {
	ID            uint   `json:"id" gorm:"primaryKey"`
	CreatedAt     int64  `json:"created_at" gorm:"index"`
	RequestID     string `json:"request_id" gorm:"index"`
	UserID        int    `json:"user_id" gorm:"index"`
	Username      string `json:"username" gorm:"index"`
	TokenID       int    `json:"token_id" gorm:"index"`
	ChannelID     int    `json:"channel_id" gorm:"index"`
	ModelName     string `json:"model_name" gorm:"index"`
	Group         string `json:"group" gorm:"index"`
	Endpoint      string `json:"endpoint"`
	Decision      string `json:"decision" gorm:"index"`
	RuleID        string `json:"rule_id" gorm:"index"`
	RuleName      string `json:"rule_name"`
	Category      string `json:"category" gorm:"index"`
	Layer         string `json:"layer" gorm:"index"`
	Severity      string `json:"severity" gorm:"index"`
	Source        string `json:"source"`
	RuleVersion   string `json:"rule_version"`
	PatternDigest string `json:"pattern_digest"`
	InputDigest   string `json:"input_digest"`
	MatchCount    int    `json:"match_count"`
}

type AdvancedSecurityEventMatch struct {
	RuleID        string
	RuleName      string
	Category      string
	Layer         string
	Severity      string
	Source        string
	RuleVersion   string
	PatternDigest string
}

type AdvancedSecurityEventParams struct {
	CreatedAt   int64
	RequestID   string
	UserID      int
	Username    string
	TokenID     int
	ChannelID   int
	ModelName   string
	Group       string
	Endpoint    string
	Decision    string
	InputDigest string
	Matches     []AdvancedSecurityEventMatch
}

func RecordAdvancedSecurityEvents(ctx context.Context, params AdvancedSecurityEventParams) error {
	if DB == nil {
		return errors.New("database is not initialized")
	}
	if len(params.Matches) == 0 {
		return nil
	}
	if ctx == nil {
		ctx = context.Background()
	}
	if params.CreatedAt <= 0 {
		params.CreatedAt = common.GetTimestamp()
	}
	if params.Decision != AdvancedSecurityDecisionBlocked && params.Decision != AdvancedSecurityDecisionAudited {
		return errors.New("invalid advanced security decision")
	}
	rows := make([]AdvancedSecurityEvent, 0, len(params.Matches))
	for _, match := range params.Matches {
		if match.RuleID == "" || match.Category == "" {
			continue
		}
		rows = append(rows, AdvancedSecurityEvent{
			CreatedAt:     params.CreatedAt,
			RequestID:     params.RequestID,
			UserID:        params.UserID,
			Username:      params.Username,
			TokenID:       params.TokenID,
			ChannelID:     params.ChannelID,
			ModelName:     params.ModelName,
			Group:         params.Group,
			Endpoint:      params.Endpoint,
			Decision:      params.Decision,
			RuleID:        match.RuleID,
			RuleName:      match.RuleName,
			Category:      match.Category,
			Layer:         match.Layer,
			Severity:      match.Severity,
			Source:        match.Source,
			RuleVersion:   match.RuleVersion,
			PatternDigest: match.PatternDigest,
			InputDigest:   params.InputDigest,
			MatchCount:    len(params.Matches),
		})
	}
	if len(rows) == 0 {
		return nil
	}
	return DB.WithContext(ctx).Create(&rows).Error
}

type AdvancedSecurityEventFilter struct {
	StartTimestamp int64
	EndTimestamp   int64
	UserID         int
	RuleID         string
	Category       string
	Decision       string
	ModelName      string
	Limit          int
	Offset         int
}

type AdvancedSecurityStatBucket struct {
	Key   string `json:"key"`
	Count int64  `json:"count"`
}

type AdvancedSecurityStats struct {
	TotalMatches     int64
	BlockedMatches   int64
	AuditedMatches   int64
	AffectedRequests int64
	AffectedUsers    int64
	ByCategory       []AdvancedSecurityStatBucket
	ByRule           []AdvancedSecurityStatBucket
}

func ListAdvancedSecurityEvents(filter AdvancedSecurityEventFilter) ([]AdvancedSecurityEvent, int64, error) {
	if DB == nil {
		return nil, 0, errors.New("database is not initialized")
	}
	query := advancedSecurityEventQuery(DB, filter)
	var total int64
	if err := query.Model(&AdvancedSecurityEvent{}).Count(&total).Error; err != nil {
		return nil, 0, err
	}
	limit := filter.Limit
	if limit <= 0 {
		limit = 50
	}
	if limit > 200 {
		limit = 200
	}
	offset := filter.Offset
	if offset < 0 {
		offset = 0
	}
	var events []AdvancedSecurityEvent
	if err := query.Order("created_at desc, id desc").Limit(limit).Offset(offset).Find(&events).Error; err != nil {
		return nil, 0, err
	}
	return events, total, nil
}

func GetAdvancedSecurityStats(filter AdvancedSecurityEventFilter) (AdvancedSecurityStats, error) {
	if DB == nil {
		return AdvancedSecurityStats{}, errors.New("database is not initialized")
	}
	query := advancedSecurityEventQuery(DB, filter)
	stats := AdvancedSecurityStats{}
	if err := query.Model(&AdvancedSecurityEvent{}).Count(&stats.TotalMatches).Error; err != nil {
		return AdvancedSecurityStats{}, err
	}
	if err := query.Session(&gorm.Session{}).Where("decision = ?", AdvancedSecurityDecisionBlocked).Model(&AdvancedSecurityEvent{}).Count(&stats.BlockedMatches).Error; err != nil {
		return AdvancedSecurityStats{}, err
	}
	if err := query.Session(&gorm.Session{}).Where("decision = ?", AdvancedSecurityDecisionAudited).Model(&AdvancedSecurityEvent{}).Count(&stats.AuditedMatches).Error; err != nil {
		return AdvancedSecurityStats{}, err
	}
	if err := countDistinctAdvancedSecurityColumn(query, "request_id", &stats.AffectedRequests); err != nil {
		return AdvancedSecurityStats{}, err
	}
	if err := countDistinctAdvancedSecurityColumn(query, "user_id", &stats.AffectedUsers); err != nil {
		return AdvancedSecurityStats{}, err
	}
	var err error
	stats.ByCategory, err = groupAdvancedSecurityStats(query, "category")
	if err != nil {
		return AdvancedSecurityStats{}, err
	}
	stats.ByRule, err = groupAdvancedSecurityStats(query, "rule_id")
	if err != nil {
		return AdvancedSecurityStats{}, err
	}
	return stats, nil
}

func advancedSecurityEventQuery(db *gorm.DB, filter AdvancedSecurityEventFilter) *gorm.DB {
	query := db.Model(&AdvancedSecurityEvent{})
	if filter.StartTimestamp > 0 {
		query = query.Where("created_at >= ?", filter.StartTimestamp)
	}
	if filter.EndTimestamp > 0 {
		query = query.Where("created_at <= ?", filter.EndTimestamp)
	}
	if filter.UserID > 0 {
		query = query.Where("user_id = ?", filter.UserID)
	}
	if filter.RuleID != "" {
		query = query.Where("rule_id = ?", filter.RuleID)
	}
	if filter.Category != "" {
		query = query.Where("category = ?", filter.Category)
	}
	if filter.Decision != "" {
		query = query.Where("decision = ?", filter.Decision)
	}
	if filter.ModelName != "" {
		query = query.Where("model_name = ?", filter.ModelName)
	}
	return query
}

func countDistinctAdvancedSecurityColumn(query *gorm.DB, column string, destination *int64) error {
	scoped := query.Session(&gorm.Session{})
	switch column {
	case "request_id":
		scoped = scoped.Where("request_id <> ''")
	case "user_id":
		scoped = scoped.Where("user_id > 0")
	}
	var row struct {
		Count int64 `gorm:"column:count"`
	}
	if err := scoped.Select("COUNT(DISTINCT " + column + ") AS count").Scan(&row).Error; err != nil {
		return err
	}
	*destination = row.Count
	return nil
}

func groupAdvancedSecurityStats(query *gorm.DB, column string) ([]AdvancedSecurityStatBucket, error) {
	var rows []AdvancedSecurityStatBucket
	err := query.Session(&gorm.Session{}).
		Select(column + " AS key, COUNT(*) AS count").
		Where(column + " <> ''").
		Group(column).
		Order("count desc, key asc").
		Limit(100).
		Scan(&rows).Error
	return rows, err
}
