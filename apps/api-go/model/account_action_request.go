/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.
*/
package model

import (
	"errors"
	"strings"
	"time"

	"github.com/LIghtJUNction/api.lmm.best/common"

	"gorm.io/gorm"
)

const (
	AccountActionKindDisable = "disable"
	AccountActionKindAppeal  = "appeal"

	AccountActionStatusPending  = "pending"
	AccountActionStatusApproved = "approved"
	AccountActionStatusRejected = "rejected"

	minAccountActionReasonRunes = 5
	maxAccountActionTextRunes   = 2000
	minAccountActionNoteRunes   = 2
)

var (
	ErrAccountActionRequestNotFound    = errors.New("账号操作申请不存在")
	ErrAccountActionRequestReviewed    = errors.New("账号操作申请已经处理")
	ErrAccountActionRequestStatus      = errors.New("账号操作申请状态无效")
	ErrAccountActionRequestKind        = errors.New("账号操作申请类型无效")
	ErrAccountActionReasonTooShort     = errors.New("申请说明至少需要 5 个字符")
	ErrAccountActionReasonTooLong      = errors.New("申请说明不能超过 2000 个字符")
	ErrAccountActionReviewNoteTooShort = errors.New("拒绝申请时必须填写至少 2 个字符的管理员意见")
	ErrAccountActionReviewNoteTooLong  = errors.New("管理员意见不能超过 2000 个字符")
	ErrAccountActionTargetForbidden    = errors.New("无权操作该账号")
	ErrAccountActionRootProtected      = errors.New("超级管理员账号不可被禁用")
	ErrAccountActionUserState          = errors.New("当前账号状态不允许此申请")
	ErrAccountActionApprovalState      = errors.New("当前账号状态已发生变化，无法审核")
	ErrAccountActionInvalidIdentity    = errors.New("账号身份校验失败")
)

// AccountActionRequest is the durable approval queue used by assistant-originated
// disable proposals and user appeals. No row in this table directly changes a
// user's state; only ReviewAccountActionRequest may apply the approved action.
type AccountActionRequest struct {
	Id                int    `json:"id" gorm:"primaryKey"`
	TargetUserId      int    `json:"target_user_id" gorm:"not null;index"`
	RequestedByUserId int    `json:"requested_by_user_id" gorm:"not null;index"`
	Kind              string `json:"kind" gorm:"type:varchar(20);not null;index"`
	Status            string `json:"status" gorm:"type:varchar(20);not null;index"`
	Reason            string `json:"reason" gorm:"type:text;not null"`
	AdminUserId       int    `json:"admin_user_id" gorm:"index"`
	AdminNote         string `json:"admin_note" gorm:"type:text"`
	CreatedAt         int64  `json:"created_at" gorm:"not null;index"`
	ReviewedAt        int64  `json:"reviewed_at" gorm:"not null;default:0"`
	Created           bool   `json:"-" gorm:"-"`
}

func (AccountActionRequest) TableName() string { return "account_action_requests" }

// AccountActionRequestView deliberately returns only operational identity
// fields. Passwords, access tokens and authentication material never enter
// the admin queue response.
type AccountActionRequestView struct {
	AccountActionRequest
	TargetUsername      string `json:"target_username"`
	TargetEmail         string `json:"target_email"`
	RequestedByUsername string `json:"requested_by_username"`
	RequestedByEmail    string `json:"requested_by_email"`
}

func normalizeAccountActionText(value string, minimum int) (string, error) {
	value = strings.TrimSpace(value)
	length := len([]rune(value))
	if length > maxAccountActionTextRunes {
		return "", ErrAccountActionReasonTooLong
	}
	if length < minimum {
		return "", ErrAccountActionReasonTooShort
	}
	return redactAssistantHandoffMessage(value), nil
}

func normalizeAccountActionReviewNote(value string, required bool) (string, error) {
	value = strings.TrimSpace(value)
	length := len([]rune(value))
	if length > maxAccountActionTextRunes {
		return "", ErrAccountActionReviewNoteTooLong
	}
	if required && length < minAccountActionNoteRunes {
		return "", ErrAccountActionReviewNoteTooShort
	}
	return value, nil
}

func validateAccountActionKind(kind string) error {
	if kind != AccountActionKindDisable && kind != AccountActionKindAppeal {
		return ErrAccountActionRequestKind
	}
	return nil
}

// SubmitAccountDisableRequest creates an approval-only disable proposal. A
// common user can only target itself; an administrator may target a lower
// privilege account. Root accounts are always protected, including when a
// root administrator is the reviewer.
func SubmitAccountDisableRequest(requestedByUserID, targetUserID int, reason string) (*AccountActionRequest, error) {
	if requestedByUserID <= 0 || targetUserID <= 0 {
		return nil, ErrAccountActionTargetForbidden
	}
	normalizedReason, err := normalizeAccountActionText(reason, minAccountActionReasonRunes)
	if err != nil {
		return nil, err
	}

	var request AccountActionRequest
	err = DB.Transaction(func(tx *gorm.DB) error {
		var requester User
		if err := lockForUpdate(tx).Where("id = ?", requestedByUserID).First(&requester).Error; err != nil {
			return err
		}
		var target User
		if err := lockForUpdate(tx).Where("id = ?", targetUserID).First(&target).Error; err != nil {
			return err
		}
		if target.Role == common.RoleRootUser {
			return ErrAccountActionRootProtected
		}
		if requester.Role < common.RoleAdminUser && targetUserID != requester.Id {
			return ErrAccountActionTargetForbidden
		}
		if requester.Role < common.RoleRootUser && target.Role >= common.RoleAdminUser {
			return ErrAccountActionTargetForbidden
		}

		var pending AccountActionRequest
		findErr := lockForUpdate(tx).
			Where("target_user_id = ? AND kind = ? AND status = ?", targetUserID, AccountActionKindDisable, AccountActionStatusPending).
			Order("id DESC").First(&pending).Error
		if findErr == nil {
			pending.Created = false
			request = pending
			return nil
		}
		if !errors.Is(findErr, gorm.ErrRecordNotFound) {
			return findErr
		}

		request = AccountActionRequest{
			TargetUserId:      targetUserID,
			RequestedByUserId: requestedByUserID,
			Kind:              AccountActionKindDisable,
			Status:            AccountActionStatusPending,
			Reason:            normalizedReason,
			CreatedAt:         common.GetTimestamp(),
			Created:           true,
		}
		return tx.Create(&request).Error
	})
	if err != nil {
		return nil, err
	}
	return &request, nil
}

// SubmitAccountAppeal accepts an appeal only for a currently disabled account.
// It is idempotent while an appeal is pending, preventing repeated clicks from
// generating multiple administrator notifications.
func SubmitAccountAppeal(userID int, reason string) (*AccountActionRequest, error) {
	if userID <= 0 {
		return nil, ErrAccountActionInvalidIdentity
	}
	normalizedReason, err := normalizeAccountActionText(reason, minAccountActionReasonRunes)
	if err != nil {
		return nil, err
	}

	var request AccountActionRequest
	err = DB.Transaction(func(tx *gorm.DB) error {
		var user User
		if err := lockForUpdate(tx).Where("id = ?", userID).First(&user).Error; err != nil {
			return err
		}
		if user.Status != common.UserStatusDisabled {
			return ErrAccountActionUserState
		}
		var pending AccountActionRequest
		findErr := lockForUpdate(tx).
			Where("target_user_id = ? AND kind = ? AND status = ?", userID, AccountActionKindAppeal, AccountActionStatusPending).
			Order("id DESC").First(&pending).Error
		if findErr == nil {
			pending.Created = false
			request = pending
			return nil
		}
		if !errors.Is(findErr, gorm.ErrRecordNotFound) {
			return findErr
		}

		request = AccountActionRequest{
			TargetUserId:      userID,
			RequestedByUserId: userID,
			Kind:              AccountActionKindAppeal,
			Status:            AccountActionStatusPending,
			Reason:            normalizedReason,
			CreatedAt:         common.GetTimestamp(),
			Created:           true,
		}
		return tx.Create(&request).Error
	})
	if err != nil {
		return nil, err
	}
	return &request, nil
}

func GetLatestAccountActionRequest(userID int, kind string) (*AccountActionRequest, error) {
	if userID <= 0 {
		return nil, gorm.ErrInvalidData
	}
	if err := validateAccountActionKind(kind); err != nil {
		return nil, err
	}
	var request AccountActionRequest
	err := DB.Where("target_user_id = ? AND kind = ?", userID, kind).Order("id DESC").First(&request).Error
	if errors.Is(err, gorm.ErrRecordNotFound) {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	return &request, nil
}

func ListAccountActionRequests(status, kind string, limit int) ([]AccountActionRequestView, error) {
	if limit <= 0 || limit > 200 {
		limit = 100
	}
	if status = strings.TrimSpace(status); status != "" && status != AccountActionStatusPending && status != AccountActionStatusApproved && status != AccountActionStatusRejected {
		return nil, ErrAccountActionRequestStatus
	}
	if kind = strings.TrimSpace(kind); kind != "" {
		if err := validateAccountActionKind(kind); err != nil {
			return nil, err
		}
	}

	query := DB.Table("account_action_requests AS request").
		Select(`request.id, request.target_user_id, request.requested_by_user_id,
			request.kind, request.status, request.reason, request.admin_user_id,
			request.admin_note, request.created_at, request.reviewed_at,
			target.username AS target_username, target.email AS target_email,
			requester.username AS requested_by_username, requester.email AS requested_by_email`).
		Joins("JOIN users AS target ON target.id = request.target_user_id AND target.deleted_at IS NULL").
		Joins("LEFT JOIN users AS requester ON requester.id = request.requested_by_user_id AND requester.deleted_at IS NULL").
		Order("request.id DESC").Limit(limit)
	if status != "" {
		query = query.Where("request.status = ?", status)
	}
	if kind != "" {
		query = query.Where("request.kind = ?", kind)
	}
	var requests []AccountActionRequestView
	if err := query.Find(&requests).Error; err != nil {
		return nil, err
	}
	return requests, nil
}

// revokeAccountSessionsWithTx changes every active session while the account
// row and request row are locked. Cache tombstones are published after the
// transaction commits by ReviewAccountActionRequest.
func revokeAccountSessionsWithTx(tx *gorm.DB, userID int, reason string, now int64) ([]UserSession, error) {
	var sessions []UserSession
	if err := tx.Where("user_id = ? AND status = ? AND expires_at > ?", userID, UserSessionStatusActive, now).Find(&sessions).Error; err != nil {
		return nil, err
	}
	if len(sessions) == 0 {
		return sessions, nil
	}
	for index := range sessions {
		if err := writeUserSessionDenyFence(&sessions[index], UserSessionStatusRevoking, now, reason); err != nil {
			return nil, err
		}
	}
	result := tx.Model(&UserSession{}).
		Where("user_id = ? AND status = ? AND expires_at > ?", userID, UserSessionStatusActive, now).
		Updates(map[string]interface{}{
			"status":         UserSessionStatusRevoked,
			"revoked_at":     now,
			"revoked_reason": reason,
		})
	if result.Error != nil {
		return nil, result.Error
	}
	for index := range sessions {
		sessions[index].Status = UserSessionStatusRevoked
		sessions[index].RevokedAt = now
		sessions[index].RevokedReason = reason
	}
	return sessions, nil
}

func applyAccountActionCacheInvalidation(userID int, authVersion int64, sessions []UserSession, tokens []Token) {
	if err := publishCommittedUserAuthVersion(userID, authVersion); err != nil {
		common.SysLog("failed to publish account action auth version: " + err.Error())
	}
	if err := PublishUserAuthCache(userID); err != nil {
		common.SysLog("failed to publish account action user cache: " + err.Error())
	}
	for index := range sessions {
		if err := writeUserSessionCache(sessions[index].cacheEntry(), time.Time{}); err != nil {
			common.SysLog("failed to publish revoked account session cache: " + err.Error())
		}
	}
	if err := invalidateTokensCache(tokens); err != nil {
		common.SysLog("failed to invalidate account token cache: " + err.Error())
	}
}

// ReviewAccountActionRequest is the only path by which an account-action row
// changes a user's status. The user and request are locked in one transaction;
// auth_version, session revocation and token disabling are committed together.
func ReviewAccountActionRequest(adminUserID, adminRole, requestID int, approve bool, note string) (*AccountActionRequest, error) {
	if adminUserID <= 0 || requestID <= 0 || adminRole < common.RoleAdminUser {
		return nil, ErrAccountActionTargetForbidden
	}
	normalizedNote, err := normalizeAccountActionReviewNote(note, !approve)
	if err != nil {
		return nil, err
	}

	var request AccountActionRequest
	var sessions []UserSession
	var tokens []Token
	var nextAuthVersion int64
	securityChanged := false
	err = DB.Transaction(func(tx *gorm.DB) error {
		if err := lockForUpdate(tx).Where("id = ?", requestID).First(&request).Error; err != nil {
			if errors.Is(err, gorm.ErrRecordNotFound) {
				return ErrAccountActionRequestNotFound
			}
			return err
		}
		if request.Status != AccountActionStatusPending {
			return ErrAccountActionRequestReviewed
		}
		if err := validateAccountActionKind(request.Kind); err != nil {
			return err
		}

		var target User
		if err := lockForUpdate(tx).Where("id = ?", request.TargetUserId).First(&target).Error; err != nil {
			if errors.Is(err, gorm.ErrRecordNotFound) {
				return ErrAccountActionRequestNotFound
			}
			return err
		}
		if target.Role == common.RoleRootUser {
			return ErrAccountActionRootProtected
		}
		if adminRole < common.RoleRootUser && target.Role >= common.RoleAdminUser {
			return ErrAccountActionTargetForbidden
		}
		if !approve {
			return updateAccountActionRequestReview(tx, &request, adminUserID, AccountActionStatusRejected, normalizedNote)
		}

		now := common.GetTimestamp()
		reason := "account_action_" + request.Kind
		sessions, err = revokeAccountSessionsWithTx(tx, target.Id, reason, now)
		if err != nil {
			return err
		}
		if err := tx.Where("user_id = ? AND status = ?", target.Id, common.TokenStatusEnabled).
			Find(&tokens).Error; err != nil {
			return err
		}
		if err := tx.Model(&Token{}).
			Where("user_id = ? AND status = ?", target.Id, common.TokenStatusEnabled).
			Update("status", common.TokenStatusDisabled).Error; err != nil {
			return err
		}

		nextAuthVersion, err = IncrementUserAuthVersionWithTx(tx, target.Id)
		if err != nil {
			return err
		}
		securityChanged = true
		newStatus := common.UserStatusDisabled
		if request.Kind == AccountActionKindAppeal {
			if target.Status != common.UserStatusDisabled {
				return ErrAccountActionApprovalState
			}
			newStatus = common.UserStatusEnabled
		}
		if err := tx.Model(&User{}).Where("id = ?", target.Id).Update("status", newStatus).Error; err != nil {
			return err
		}
		return updateAccountActionRequestReview(tx, &request, adminUserID, AccountActionStatusApproved, normalizedNote)
	})
	if err != nil {
		return nil, err
	}
	if securityChanged {
		applyAccountActionCacheInvalidation(request.TargetUserId, nextAuthVersion, sessions, tokens)
	}
	return &request, nil
}

func updateAccountActionRequestReview(tx *gorm.DB, request *AccountActionRequest, adminUserID int, status, note string) error {
	now := common.GetTimestamp()
	if err := tx.Model(request).Updates(map[string]interface{}{
		"status":        status,
		"admin_user_id": adminUserID,
		"admin_note":    note,
		"reviewed_at":   now,
	}).Error; err != nil {
		return err
	}
	request.Status = status
	request.AdminUserId = adminUserID
	request.AdminNote = note
	request.ReviewedAt = now
	return nil
}
