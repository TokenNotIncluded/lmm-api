package model

import (
	"strings"

	"gorm.io/gorm"
)

// DeveloperAccessRecommendationArchive is an immutable snapshot of the
// recommendation that supported a successful L1 approval. The active request
// is intentionally reused while a user edits a pending letter; this table is
// the durable audit history and is never overwritten by later edits.
type DeveloperAccessRecommendationArchive struct {
	Id             int    `json:"id" gorm:"primaryKey"`
	UserId         int    `json:"user_id" gorm:"not null;index"`
	RequestId      int    `json:"request_id" gorm:"not null;index"`
	Source         string `json:"source" gorm:"type:varchar(40);not null"`
	Reason         string `json:"reason" gorm:"type:text;not null"`
	Recommendation string `json:"recommendation" gorm:"type:text;not null"`
	AdminUserId    int    `json:"admin_user_id" gorm:"not null;index"`
	AdminNote      string `json:"admin_note" gorm:"type:text"`
	ApprovedAt     int64  `json:"approved_at" gorm:"not null;index"`
	CreatedAt      int64  `json:"created_at" gorm:"not null"`
}

type recommendationArchiveIdentity struct {
	requestID  int
	approvedAt int64
}

func (DeveloperAccessRecommendationArchive) TableName() string {
	return "developer_access_recommendation_archives"
}

// ListDeveloperAccessRecommendationArchives returns bounded, newest-first
// history for an administrator's permitted target. It deliberately exposes
// only letter/review fields; no conversation transcript or private card data
// is joined into this optional view.
func ListDeveloperAccessRecommendationArchives(userID int, limit int) ([]DeveloperAccessRecommendationArchive, error) {
	if userID <= 0 {
		return nil, gorm.ErrInvalidData
	}
	if limit <= 0 || limit > 100 {
		limit = 50
	}
	var rows []DeveloperAccessRecommendationArchive
	err := DB.Where("user_id = ?", userID).
		Order("approved_at DESC, id DESC").
		Limit(limit).
		Find(&rows).Error
	return rows, err
}

func archiveApprovedDeveloperAccessRecommendation(tx *gorm.DB, request DeveloperAccessRequest) error {
	recommendation := strings.TrimSpace(request.AIRecommendation)
	if recommendation == "" {
		return nil
	}
	return tx.Create(&DeveloperAccessRecommendationArchive{
		UserId:         request.UserId,
		RequestId:      request.Id,
		Source:         request.Source,
		Reason:         request.Reason,
		Recommendation: recommendation,
		AdminUserId:    request.AdminUserId,
		AdminNote:      request.AdminNote,
		ApprovedAt:     request.ReviewedAt,
		CreatedAt:      request.ReviewedAt,
	}).Error
}

// BackfillDeveloperAccessRecommendationArchives makes the archive feature
// useful immediately after upgrade for already-approved requests. The check
// uses the immutable approval timestamp so a later re-approval of a reopened
// request receives its own snapshot.
func BackfillDeveloperAccessRecommendationArchives() error {
	const batchSize = 200
	return DB.Transaction(func(tx *gorm.DB) error {
		var requests []DeveloperAccessRequest
		return tx.Where("status = ? AND TRIM(ai_recommendation) <> ''", DeveloperAccessRequestApproved).
			FindInBatches(&requests, batchSize, func(batchTx *gorm.DB, _ int) error {
				requestIDs := make([]int, 0, len(requests))
				for _, request := range requests {
					requestIDs = append(requestIDs, request.Id)
				}
				var existing []DeveloperAccessRecommendationArchive
				if err := batchTx.Where("request_id IN ?", requestIDs).Find(&existing).Error; err != nil {
					return err
				}
				existingKeys := make(map[recommendationArchiveIdentity]struct{}, len(existing))
				for _, archive := range existing {
					existingKeys[recommendationArchiveIdentity{requestID: archive.RequestId, approvedAt: archive.ApprovedAt}] = struct{}{}
				}
				for _, request := range requests {
					if _, exists := existingKeys[recommendationArchiveIdentity{requestID: request.Id, approvedAt: request.ReviewedAt}]; exists {
						continue
					}
					if err := archiveApprovedDeveloperAccessRecommendation(batchTx, request); err != nil {
						return err
					}
				}
				return nil
			}).Error
	})
}
