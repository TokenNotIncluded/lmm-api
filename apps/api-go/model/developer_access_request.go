package model

import (
	"errors"
	"strings"
	"time"

	"github.com/QuantumNous/new-api/common"
	"gorm.io/gorm"
)

const (
	DeveloperAccessRequestPending         = "pending"
	DeveloperAccessRequestApproved        = "approved"
	DeveloperAccessRequestRejected        = "rejected"
	DeveloperAccessRequestSourceAI        = "assistant_recommendation"
	DeveloperAccessRequestSourceAssistant = "assistant_request"
	DeveloperAccessRequestSourceOld       = "legacy"
	minDeveloperAccessRequestReason       = 5
	minDeveloperAccessReviewNote          = 2
	minDeveloperAccessRecommendation      = 20
	maxDeveloperAccessRequestNote         = 2000
)

var (
	ErrDeveloperAccessRequestNotFound         = errors.New("解锁申请不存在")
	ErrDeveloperAccessRequestReviewed         = errors.New("解锁申请已经处理")
	ErrDeveloperAccessRequestStatus           = errors.New("解锁申请状态无效")
	ErrDeveloperAccessRequestReasonTooShort   = errors.New("解锁申请说明至少需要 5 个字符")
	ErrDeveloperAccessRecommendationTooShort  = errors.New("AI 推荐信至少需要 20 个字符")
	ErrDeveloperAccessReviewNoteTooShort      = errors.New("管理员意见至少需要 2 个字符")
	ErrDeveloperAccessRequestNoteTooLong      = errors.New("解锁申请说明不能超过 2000 个字符")
	ErrDeveloperAccessRequestQueueUnavailable = errors.New("解锁申请队列暂时不可用")
)

// DeveloperAccessRequest records the non-payment path to L1 access. The
// request is deliberately separate from User.TrustLevelOverride: approving a
// request unlocks L1 without freezing later paid progression at that level.
type DeveloperAccessRequest struct {
	Id               int    `json:"id" gorm:"primaryKey"`
	UserId           int    `json:"user_id" gorm:"not null;index"`
	Status           string `json:"status" gorm:"type:varchar(20);not null;index"`
	Source           string `json:"source" gorm:"type:varchar(40);not null;default:legacy;index"`
	Reason           string `json:"reason" gorm:"type:text"`
	AIRecommendation string `json:"ai_recommendation" gorm:"type:text"`
	AdminUserId      int    `json:"admin_user_id" gorm:"index"`
	AdminNote        string `json:"admin_note" gorm:"type:text"`
	CreatedAt        int64  `json:"created_at" gorm:"not null;index"`
	ReviewedAt       int64  `json:"reviewed_at" gorm:"not null;default:0"`
}

func (DeveloperAccessRequest) TableName() string { return "developer_access_requests" }

type DeveloperAccessRequestView struct {
	DeveloperAccessRequest
	Username string `json:"username"`
	Email    string `json:"email"`
}

func normalizeDeveloperAccessRequestText(value string) (string, error) {
	value = strings.TrimSpace(value)
	if len([]rune(value)) > maxDeveloperAccessRequestNote {
		return "", ErrDeveloperAccessRequestNoteTooLong
	}
	return value, nil
}

func normalizeDeveloperAccessRequestReason(value string) (string, error) {
	value, err := normalizeDeveloperAccessRequestText(value)
	if err != nil {
		return "", err
	}
	if len([]rune(value)) < minDeveloperAccessRequestReason {
		return "", ErrDeveloperAccessRequestReasonTooShort
	}
	return value, nil
}

func normalizeDeveloperAccessRecommendation(value string) (string, error) {
	value, err := normalizeDeveloperAccessRequestText(value)
	if err != nil {
		return "", err
	}
	if len([]rune(value)) < minDeveloperAccessRecommendation {
		return "", ErrDeveloperAccessRecommendationTooShort
	}
	return redactAssistantHandoffMessage(value), nil
}

func normalizeDeveloperAccessReviewNote(value string) (string, error) {
	value, err := normalizeDeveloperAccessRequestText(value)
	if err != nil {
		return "", err
	}
	if len([]rune(value)) < minDeveloperAccessReviewNote {
		return "", ErrDeveloperAccessReviewNoteTooShort
	}
	return value, nil
}

func GetDeveloperAccessRequest(userID int) (*DeveloperAccessRequest, error) {
	if userID <= 0 {
		return nil, gorm.ErrInvalidData
	}
	var request DeveloperAccessRequest
	err := DB.Where("user_id = ?", userID).Order("id DESC").First(&request).Error
	if errors.Is(err, gorm.ErrRecordNotFound) {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	return &request, nil
}

// reopenDeveloperAccessRequestForUserWithTx makes an explicit administrator L0
// transition visible as a fresh application state without rewriting the
// reviewed row. The previous approved request remains an audit record, while
// the newest row is the one returned to the user and can be reviewed again.
func reopenDeveloperAccessRequestForUserWithTx(tx *gorm.DB, userID int) error {
	var latest DeveloperAccessRequest
	err := lockForUpdate(tx).Where("user_id = ?", userID).Order("id DESC").First(&latest).Error
	if errors.Is(err, gorm.ErrRecordNotFound) {
		return nil
	}
	if err != nil {
		return err
	}
	if latest.Status != DeveloperAccessRequestApproved {
		return nil
	}

	source := latest.Source
	if source == "" {
		source = DeveloperAccessRequestSourceOld
	}
	return tx.Create(&DeveloperAccessRequest{
		UserId:           userID,
		Status:           DeveloperAccessRequestPending,
		Source:           source,
		Reason:           latest.Reason,
		AIRecommendation: latest.AIRecommendation,
		CreatedAt:        common.GetTimestamp(),
	}).Error
}

func SubmitDeveloperAccessRequest(userID int, reason string) (*DeveloperAccessRequest, error) {
	return submitDeveloperAccessRequest(userID, reason, "", DeveloperAccessRequestSourceOld)
}

// SubmitAssistantDeveloperAccessRequest puts a concrete L1 request in the
// administrator queue before any model/tool call. An AI recommendation is an
// optional review aid, never a prerequisite for being seen by an operator.
func SubmitAssistantDeveloperAccessRequest(userID int, reason string) (*DeveloperAccessRequest, error) {
	return submitDeveloperAccessRequest(userID, reason, "", DeveloperAccessRequestSourceAssistant)
}

func SubmitAssistantDeveloperAccessRecommendation(userID int, reason string, recommendation string) (*DeveloperAccessRequest, error) {
	return submitDeveloperAccessRequest(userID, reason, recommendation, DeveloperAccessRequestSourceAI)
}

func submitDeveloperAccessRequest(userID int, reason string, recommendation string, source string) (*DeveloperAccessRequest, error) {
	if userID <= 0 {
		return nil, gorm.ErrInvalidData
	}
	normalizedReason, err := normalizeDeveloperAccessRequestReason(reason)
	if err != nil {
		return nil, err
	}
	normalizedRecommendation := ""
	if source == DeveloperAccessRequestSourceAI {
		normalizedRecommendation, err = normalizeDeveloperAccessRecommendation(recommendation)
		if err != nil {
			return nil, err
		}
	}

	var request DeveloperAccessRequest
	err = DB.Transaction(func(tx *gorm.DB) error {
		// Lock the user row before checking for a pending request. This makes
		// duplicate submissions from two browser tabs collapse to one request
		// on databases that support row-level locks.
		var user User
		if err := lockForUpdate(tx).Where("id = ?", userID).First(&user).Error; err != nil {
			return err
		}
		var pending DeveloperAccessRequest
		findErr := tx.Where("user_id = ? AND status = ?", userID, DeveloperAccessRequestPending).
			Order("id DESC").First(&pending).Error
		if findErr == nil {
			// A request is visible as soon as it is submitted. If a later AI
			// turn produces a recommendation, enrich that same pending row
			// instead of creating a second queue item or making the user start
			// over. Never replace the user's original statement.
			if source == DeveloperAccessRequestSourceAI && strings.TrimSpace(pending.AIRecommendation) == "" {
				if err := tx.Model(&pending).Updates(map[string]interface{}{
					"ai_recommendation": normalizedRecommendation,
					"source":            source,
				}).Error; err != nil {
					return err
				}
				pending.AIRecommendation = normalizedRecommendation
				pending.Source = source
			}
			request = pending
			return nil
		}
		if !errors.Is(findErr, gorm.ErrRecordNotFound) {
			return findErr
		}
		request = DeveloperAccessRequest{
			UserId:           userID,
			Status:           DeveloperAccessRequestPending,
			Source:           source,
			Reason:           redactAssistantHandoffMessage(normalizedReason),
			AIRecommendation: normalizedRecommendation,
			CreatedAt:        common.GetTimestamp(),
		}
		return tx.Create(&request).Error
	})
	if err != nil {
		// Keep the database cause for diagnostics while giving HTTP callers a
		// stable classification. They must not treat a failed queue write as a
		// successful chat turn and should retry the same request instead.
		return nil, errors.Join(ErrDeveloperAccessRequestQueueUnavailable, err)
	}
	return &request, nil
}

func ListDeveloperAccessRequests(status string, limit int) ([]DeveloperAccessRequestView, error) {
	if limit <= 0 || limit > 200 {
		limit = 100
	}
	query := DB.Table("developer_access_requests AS request").
		Select("request.*, users.username, users.email").
		Joins("JOIN users ON users.id = request.user_id").
		Order("request.id DESC").Limit(limit)
	if status = strings.TrimSpace(status); status != "" {
		if status != DeveloperAccessRequestPending && status != DeveloperAccessRequestApproved && status != DeveloperAccessRequestRejected {
			return nil, ErrDeveloperAccessRequestStatus
		}
		query = query.Where("request.status = ?", status)
	}
	var requests []DeveloperAccessRequestView
	if err := query.Find(&requests).Error; err != nil {
		return nil, err
	}
	return requests, nil
}

func ReviewDeveloperAccessRequest(adminUserID int, requestID int, approve bool, note string) (*DeveloperAccessRequest, error) {
	if adminUserID <= 0 || requestID <= 0 {
		return nil, gorm.ErrInvalidData
	}
	normalizedNote, err := normalizeDeveloperAccessReviewNote(note)
	if err != nil {
		return nil, err
	}
	var request DeveloperAccessRequest
	err = DB.Transaction(func(tx *gorm.DB) error {
		if err := lockForUpdate(tx).Where("id = ?", requestID).First(&request).Error; err != nil {
			if errors.Is(err, gorm.ErrRecordNotFound) {
				return ErrDeveloperAccessRequestNotFound
			}
			return err
		}
		if request.Status != DeveloperAccessRequestPending {
			return ErrDeveloperAccessRequestReviewed
		}
		if approve {
			// This timestamp is the durable non-payment activation fact. It
			// grants L1, while paid top-ups can still raise the automatic level.
			result := tx.Model(&User{}).
				Where("id = ?", request.UserId).
				Updates(map[string]interface{}{
					"console_activated_at": time.Now().Unix(),
					// A previous explicit L0 test reset must not survive a new
					// administrator approval.
					"trust_level_override": nil,
				})
			if result.Error != nil {
				return result.Error
			}
			if result.RowsAffected == 0 {
				var user User
				if err := tx.Select("id").First(&user, request.UserId).Error; err != nil {
					if errors.Is(err, gorm.ErrRecordNotFound) {
						return ErrDeveloperAccessRequestNotFound
					}
					return err
				}
			}
		}
		now := common.GetTimestamp()
		status := DeveloperAccessRequestRejected
		if approve {
			status = DeveloperAccessRequestApproved
		}
		if err := tx.Model(&request).Updates(map[string]interface{}{
			"status":        status,
			"admin_user_id": adminUserID,
			"admin_note":    normalizedNote,
			"reviewed_at":   now,
		}).Error; err != nil {
			return err
		}
		request.Status = status
		request.AdminUserId = adminUserID
		request.AdminNote = normalizedNote
		request.ReviewedAt = now
		return nil
	})
	if err != nil {
		return nil, err
	}
	if approve {
		_ = InvalidateUserCache(request.UserId)
	}
	return &request, nil
}
