package model

import (
	"errors"
	"math"
	"net"
	"strings"
	"time"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"gorm.io/gorm"
	"gorm.io/gorm/clause"
)

const (
	AssistantGiftOffered  = "offered"
	AssistantGiftClaimed  = "claimed"
	AssistantGiftDeclined = "declined"
	assistantGiftMaxCents = 1000
	assistantGiftMaxAge   = 30 * 24 * time.Hour
	assistantGiftRiskAge  = 30 * 24 * time.Hour
	assistantGiftIPLimit  = 3
)

var (
	ErrAssistantGiftIneligible  = errors.New("new-user assistant gift is not available")
	ErrAssistantGiftInvalid     = errors.New("new-user assistant gift decision is invalid")
	ErrAssistantGiftUnavailable = errors.New("new-user assistant gift cannot be claimed")
	ErrAssistantGiftAbuse       = errors.New("new-user assistant gift risk limit reached")
)

const (
	assistantGiftRiskIdentity = "identity"
	assistantGiftRiskNetwork  = "network"
)

// AssistantGiftRiskMemory is the privacy-minimized global fraud ledger for
// welcome gifts. KeyHash is an HMAC of either a provider-aware email identity
// or a normalized IP address; raw identifiers and transcripts are never
// stored or exposed to the assistant. Identity rows never reset. Network rows
// reuse one bounded counter per address and roll forward every 30 days.
type AssistantGiftRiskMemory struct {
	KeyHash         string `json:"-" gorm:"type:char(64);primaryKey"`
	Kind            string `json:"-" gorm:"type:varchar(16);not null;index"`
	DecisionCount   int    `json:"-" gorm:"not null;default:0"`
	WindowStartedAt int64  `json:"-" gorm:"not null"`
	UpdatedAt       int64  `json:"-" gorm:"not null;index"`
}

func (AssistantGiftRiskMemory) TableName() string { return "assistant_gift_risk_memories" }

// AssistantNewUserGift is a one-time, user-scoped decision. AmountCents and
// Quota are both persisted so a later exchange-rate change cannot alter an
// already presented gift. A zero-dollar decision is retained as declined and
// consumes the same single opportunity.
type AssistantNewUserGift struct {
	Id             int64  `json:"id" gorm:"primaryKey"`
	UserId         int    `json:"-" gorm:"not null;uniqueIndex"`
	ConversationId int64  `json:"conversation_id,omitempty" gorm:"not null;default:0;index"`
	AmountCents    int    `json:"amount_cents" gorm:"not null"`
	Quota          int    `json:"quota" gorm:"not null"`
	Status         string `json:"status" gorm:"type:varchar(20);not null;index"`
	Reason         string `json:"reason" gorm:"type:varchar(240);not null"`
	CreatedAt      int64  `json:"created_at" gorm:"not null;index"`
	ClaimedAt      int64  `json:"claimed_at" gorm:"not null;default:0"`
}

func (AssistantNewUserGift) TableName() string { return "assistant_new_user_gifts" }

func GetAssistantNewUserGift(userID int) (*AssistantNewUserGift, error) {
	if userID <= 0 {
		return nil, gorm.ErrInvalidData
	}
	var gift AssistantNewUserGift
	err := DB.Where("user_id = ?", userID).First(&gift).Error
	if errors.Is(err, gorm.ErrRecordNotFound) {
		return nil, nil
	}
	return &gift, err
}

// DecideAssistantNewUserGift persists the model's bounded decision exactly
// once. Conversation evidence is calculated by trusted controller code; it is
// not accepted from a browser. The database uniqueness constraint is the
// final race boundary across retries and instances.
func DecideAssistantNewUserGift(userID int, conversationID int64, amountCents int, reason string, substantiveTurns int, substantiveRunes int, clientIP string) (*AssistantNewUserGift, bool, error) {
	if userID <= 0 || conversationID < 0 || amountCents < 0 || amountCents > assistantGiftMaxCents || substantiveTurns < 2 || substantiveRunes < 24 {
		return nil, false, ErrAssistantGiftInvalid
	}
	reason = strings.TrimSpace(redactAssistantHandoffMessage(reason))
	if len([]rune(reason)) < 2 || len([]rune(reason)) > 240 {
		return nil, false, ErrAssistantGiftInvalid
	}

	if existing, err := GetAssistantNewUserGift(userID); err != nil || existing != nil {
		return existing, false, err
	}

	var user User
	if err := DB.Select("id", "email", "role", "status", "created_at", "last_api_activity_at", "trust_level_override", "console_activated_at").First(&user, "id = ?", userID).Error; err != nil {
		return nil, false, err
	}
	now := time.Now()
	created := time.Unix(user.CreatedAt, 0)
	if user.Role != common.RoleCommonUser || user.Status != common.UserStatusEnabled || user.CreatedAt <= 0 || created.After(now) || now.Sub(created) > assistantGiftMaxAge || strings.TrimSpace(user.Email) == "" || IsDisposableEmail(user.Email) {
		return nil, false, ErrAssistantGiftIneligible
	}
	access, err := GetFreshUserAccessSnapshot(&user)
	if err != nil {
		return nil, false, err
	}
	if access.DeveloperAccess.Granted || access.TrustLevel.Level > TrustLevelMinUser {
		return nil, false, ErrAssistantGiftIneligible
	}
	identityHash, networkHash, err := assistantGiftRiskKeys(user.Email, clientIP)
	if err != nil {
		return nil, false, ErrAssistantGiftIneligible
	}

	quota := int(math.Round(float64(amountCents) * common.QuotaPerUnit / 100))
	if quota < 0 || (amountCents > 0 && quota <= 0) {
		return nil, false, ErrAssistantGiftInvalid
	}
	status := AssistantGiftOffered
	if amountCents == 0 {
		status = AssistantGiftDeclined
	}
	gift := AssistantNewUserGift{
		UserId:         userID,
		ConversationId: conversationID,
		AmountCents:    amountCents,
		Quota:          quota,
		Status:         status,
		Reason:         reason,
		CreatedAt:      common.GetTimestamp(),
	}
	createdDecision := false
	err = DB.Transaction(func(tx *gorm.DB) error {
		if err := lockAssistantOwner(tx, userID); err != nil {
			return err
		}
		var existing AssistantNewUserGift
		existingErr := lockForUpdate(tx).Where("user_id = ?", userID).First(&existing).Error
		if existingErr == nil {
			gift = existing
			return nil
		}
		if !errors.Is(existingErr, gorm.ErrRecordNotFound) {
			return existingErr
		}
		nowTimestamp := common.GetTimestamp()
		if err := reserveAssistantGiftRisk(tx, identityHash, assistantGiftRiskIdentity, nowTimestamp, 1, 0); err != nil {
			return err
		}
		if err := reserveAssistantGiftRisk(tx, networkHash, assistantGiftRiskNetwork, nowTimestamp, assistantGiftIPLimit, int64(assistantGiftRiskAge/time.Second)); err != nil {
			return err
		}
		if err := tx.Create(&gift).Error; err != nil {
			return err
		}
		createdDecision = true
		return nil
	})
	if err != nil {
		return nil, false, err
	}
	return &gift, createdDecision, nil
}

func assistantGiftRiskKeys(email, clientIP string) (string, string, error) {
	identity := canonicalAssistantGiftEmail(email)
	ip := net.ParseIP(strings.TrimSpace(clientIP))
	if identity == "" || ip == nil {
		return "", "", ErrAssistantGiftInvalid
	}
	return common.GenerateHMAC("assistant-gift-identity-v1:" + identity),
		common.GenerateHMAC("assistant-gift-network-v1:" + ip.String()), nil
}

func canonicalAssistantGiftEmail(email string) string {
	email = NormalizeEmail(email)
	local, domain, found := strings.Cut(email, "@")
	if !found || local == "" || domain == "" || strings.Contains(domain, "@") {
		return ""
	}
	switch domain {
	case "googlemail.com":
		domain = "gmail.com"
		fallthrough
	case "gmail.com":
		local = strings.ReplaceAll(local, ".", "")
		if plus := strings.IndexByte(local, '+'); plus >= 0 {
			local = local[:plus]
		}
	case "hotmail.com", "live.com", "outlook.com":
		if plus := strings.IndexByte(local, '+'); plus >= 0 {
			local = local[:plus]
		}
	}
	if local == "" {
		return ""
	}
	return local + "@" + domain
}

func reserveAssistantGiftRisk(tx *gorm.DB, keyHash, kind string, now int64, limit int, windowSeconds int64) error {
	if tx == nil || keyHash == "" || limit <= 0 {
		return ErrAssistantGiftInvalid
	}
	seed := AssistantGiftRiskMemory{
		KeyHash: keyHash, Kind: kind, WindowStartedAt: now, UpdatedAt: now,
	}
	if err := tx.Clauses(clause.OnConflict{DoNothing: true}).Create(&seed).Error; err != nil {
		return err
	}
	var memory AssistantGiftRiskMemory
	if err := lockForUpdate(tx).Where("key_hash = ?", keyHash).First(&memory).Error; err != nil {
		return err
	}
	if memory.Kind != kind {
		return ErrAssistantGiftInvalid
	}
	count := memory.DecisionCount
	windowStart := memory.WindowStartedAt
	if windowSeconds > 0 && (windowStart <= 0 || now-windowStart >= windowSeconds) {
		count = 0
		windowStart = now
	}
	if count >= limit {
		return ErrAssistantGiftAbuse
	}
	return tx.Model(&AssistantGiftRiskMemory{}).Where("key_hash = ?", keyHash).Updates(map[string]any{
		"decision_count":    count + 1,
		"window_started_at": windowStart,
		"updated_at":        now,
	}).Error
}

// ClaimAssistantNewUserGift credits the stored quota and marks the gift in
// one transaction. Repeated claims return the same row without another credit.
func ClaimAssistantNewUserGift(userID int) (*AssistantNewUserGift, bool, error) {
	if userID <= 0 {
		return nil, false, gorm.ErrInvalidData
	}
	var gift AssistantNewUserGift
	alreadyClaimed := false
	err := DB.Transaction(func(tx *gorm.DB) error {
		if err := lockAssistantOwner(tx, userID); err != nil {
			return err
		}
		if err := lockForUpdate(tx).Where("user_id = ?", userID).First(&gift).Error; err != nil {
			if errors.Is(err, gorm.ErrRecordNotFound) {
				return ErrAssistantGiftUnavailable
			}
			return err
		}
		if gift.Status == AssistantGiftClaimed {
			alreadyClaimed = true
			return nil
		}
		if gift.Status != AssistantGiftOffered || gift.AmountCents <= 0 || gift.Quota <= 0 {
			return ErrAssistantGiftUnavailable
		}
		if err := tx.Model(&User{}).Where("id = ?", userID).Update("quota", gorm.Expr("quota + ?", gift.Quota)).Error; err != nil {
			return err
		}
		gift.Status = AssistantGiftClaimed
		gift.ClaimedAt = common.GetTimestamp()
		return tx.Model(&gift).Updates(map[string]any{
			"status":     gift.Status,
			"claimed_at": gift.ClaimedAt,
		}).Error
	})
	if err != nil {
		return nil, false, err
	}
	if !alreadyClaimed {
		if err := cacheIncrUserQuota(userID, int64(gift.Quota)); err != nil {
			common.SysLog("failed to update new-user assistant gift quota cache: " + err.Error())
		}
	}
	return &gift, alreadyClaimed, nil
}
