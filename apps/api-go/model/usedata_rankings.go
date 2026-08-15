package model

import (
	"database/sql"
	"fmt"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"gorm.io/gorm"
)

type RankingQuotaTotal struct {
	ModelName   string `json:"model_name"`
	TotalTokens int64  `json:"total_tokens"`
}

type RankingQuotaBucket struct {
	ModelName string `json:"model_name"`
	Bucket    int64  `json:"bucket"`
	Tokens    int64  `json:"tokens"`
}

type UserRankingTotal struct {
	UserID      int   `json:"user_id"`
	Requests    int64 `json:"requests"`
	TotalTokens int64 `json:"total_tokens"`
}

// UserRankingRow is a single database-aggregated usage row enriched with the
// small public user projection needed by the leaderboard. It is consumed as a
// cursor so callers never materialize every active user in a long window.
type UserRankingRow struct {
	UserRankingTotal
	Username    string
	DisplayName string
	Status      int
	Setting     string
}

func GetRankingQuotaTotals(startTime int64, endTime int64) ([]RankingQuotaTotal, error) {
	var rows []RankingQuotaTotal
	query := DB.Table("quota_data").
		Select("model_name, sum(token_used) as total_tokens").
		Where("model_name <> ''").
		Group("model_name").
		Having("sum(token_used) > 0").
		Order("total_tokens DESC")
	query = applyRankingQuotaTimeRange(query, startTime, endTime)
	err := query.Find(&rows).Error
	return rows, err
}

func GetRankingQuotaBuckets(startTime int64, endTime int64, bucketSize int64) ([]RankingQuotaBucket, error) {
	if bucketSize <= 0 {
		bucketSize = 3600
	}
	bucketExpr := rankingBucketExpr(bucketSize)
	var rows []RankingQuotaBucket
	query := DB.Table("quota_data").
		Select(fmt.Sprintf("model_name, %s as bucket, sum(token_used) as tokens", bucketExpr)).
		Where("model_name <> ''").
		Group(fmt.Sprintf("model_name, %s", bucketExpr)).
		Having("sum(token_used) > 0").
		Order("bucket ASC")
	query = applyRankingQuotaTimeRange(query, startTime, endTime)
	err := query.Find(&rows).Error
	return rows, err
}

func GetUserRankingTotals(startTime int64, endTime int64) ([]UserRankingTotal, error) {
	var rows []UserRankingTotal
	query := DB.Table("quota_data").
		Select("user_id, COALESCE(SUM(count), 0) AS requests, COALESCE(SUM(token_used), 0) AS total_tokens").
		Where("user_id > 0").
		Group("user_id").
		Having("COALESCE(SUM(count), 0) > 0 OR COALESCE(SUM(token_used), 0) > 0").
		Order("total_tokens DESC, requests DESC")
	query = applyRankingQuotaTimeRange(query, startTime, endTime)
	return rows, query.Find(&rows).Error
}

// IterateUserRankingRows streams grouped usage rows directly from the
// database. The slice-returning helper above remains for compatibility, while
// the public leaderboard uses this bounded path for large installations.
func IterateUserRankingRows(startTime int64, endTime int64, visit func(UserRankingRow) error) error {
	if DB == nil || visit == nil {
		return gorm.ErrInvalidData
	}
	query := DB.Table("quota_data").
		Select("quota_data.user_id, COALESCE(SUM(quota_data.count), 0) AS requests, COALESCE(SUM(quota_data.token_used), 0) AS total_tokens, users.username, users.display_name, users.status, users.setting").
		Joins("JOIN users ON users.id = quota_data.user_id").
		Where("quota_data.user_id > ?", 0).
		Group("quota_data.user_id, users.username, users.display_name, users.status, users.setting").
		Having("COALESCE(SUM(quota_data.count), 0) > 0 OR COALESCE(SUM(quota_data.token_used), 0) > 0").
		Order("total_tokens DESC, requests DESC, quota_data.user_id ASC")
	query = applyRankingQuotaTimeRange(query, startTime, endTime)
	rows, err := query.Rows()
	if err != nil {
		return err
	}
	defer rows.Close()
	for rows.Next() {
		var row UserRankingRow
		var username, displayName, setting sql.NullString
		if err := rows.Scan(
			&row.UserID,
			&row.Requests,
			&row.TotalTokens,
			&username,
			&displayName,
			&row.Status,
			&setting,
		); err != nil {
			return err
		}
		row.Username = username.String
		row.DisplayName = displayName.String
		row.Setting = setting.String
		if err := visit(row); err != nil {
			return err
		}
	}
	return rows.Err()
}

func GetUsersForUsageRanking(userIDs []int) ([]*User, error) {
	if len(userIDs) == 0 {
		return []*User{}, nil
	}

	var users []*User
	err := DB.Select("id, username, display_name, status, setting").
		Where("id IN ?", userIDs).
		Find(&users).Error
	return users, err
}

func rankingBucketExpr(bucketSize int64) string {
	if common.UsingMainDatabase(common.DatabaseTypeMySQL) {
		return fmt.Sprintf("FLOOR(created_at / %d) * %d", bucketSize, bucketSize)
	}
	return fmt.Sprintf("(created_at / %d) * %d", bucketSize, bucketSize)
}

func applyRankingQuotaTimeRange(query *gorm.DB, startTime int64, endTime int64) *gorm.DB {
	if startTime > 0 {
		query = query.Where("quota_data.created_at >= ?", startTime)
	}
	if endTime > 0 {
		query = query.Where("quota_data.created_at <= ?", endTime)
	}
	return query
}
