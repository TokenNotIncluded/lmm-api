package model

import (
	"context"
	"time"

	"gorm.io/gorm"
)

const assistantRetentionBatchMax = 500

type AssistantRetentionCutoffs struct {
	ActiveBefore     int64 `json:"active_before"`
	ArchivedBefore   int64 `json:"archived_before"`
	RestrictedBefore int64 `json:"restricted_before"`
}

type AssistantRetentionDeleteResult struct {
	Conversations  int64 `json:"conversations"`
	Messages       int64 `json:"messages"`
	SecureCards    int64 `json:"secure_cards"`
	Incidents      int64 `json:"incidents"`
	IntentLeads    int64 `json:"intent_leads"`
	ProfileAudits  int64 `json:"profile_audits"`
	SecurityEvents int64 `json:"security_events"`
}

func NormalizeAssistantRetentionBatchSize(batchSize int) int {
	if batchSize <= 0 {
		return 200
	}
	if batchSize > assistantRetentionBatchMax {
		return assistantRetentionBatchMax
	}
	return batchSize
}

func AssistantRetentionCutoffsFromNow(now time.Time, activeDays, archivedDays, restrictedDays int) AssistantRetentionCutoffs {
	return AssistantRetentionCutoffs{
		ActiveBefore:     now.Add(-time.Duration(activeDays) * 24 * time.Hour).Unix(),
		ArchivedBefore:   now.Add(-time.Duration(archivedDays) * 24 * time.Hour).Unix(),
		RestrictedBefore: now.Add(-time.Duration(restrictedDays) * 24 * time.Hour).Unix(),
	}
}

func assistantRetentionEligible(query *gorm.DB, cutoffs AssistantRetentionCutoffs) *gorm.DB {
	return query.Where(
		"(restricted_at > 0 AND restricted_at < ?) OR "+
			"(restricted_at = 0 AND archived_at > 0 AND archived_at < ?) OR "+
			"(restricted_at = 0 AND archived_at = 0 AND updated_at < ?)",
		cutoffs.RestrictedBefore,
		cutoffs.ArchivedBefore,
		cutoffs.ActiveBefore,
	)
}

// PurgeAssistantConversationsBefore deletes one bounded batch. Eligibility is
// rechecked while the rows are locked, so a conversation updated after a
// scheduler payload was created is never deleted by a stale candidate list.
func PurgeAssistantConversationsBefore(ctx context.Context, cutoffs AssistantRetentionCutoffs, batchSize int) (AssistantRetentionDeleteResult, error) {
	result := AssistantRetentionDeleteResult{}
	if cutoffs.ActiveBefore <= 0 || cutoffs.ArchivedBefore <= 0 || cutoffs.RestrictedBefore <= 0 {
		return result, gorm.ErrInvalidData
	}
	batchSize = NormalizeAssistantRetentionBatchSize(batchSize)

	err := DB.WithContext(ctx).Transaction(func(tx *gorm.DB) error {
		var conversations []AssistantConversation
		query := lockForUpdate(tx.WithContext(ctx)).Model(&AssistantConversation{}).
			Select("id").Order("id ASC").Limit(batchSize)
		if err := assistantRetentionEligible(query, cutoffs).Find(&conversations).Error; err != nil {
			return err
		}
		if len(conversations) == 0 {
			return nil
		}

		conversationIDs := make([]int64, 0, len(conversations))
		for _, conversation := range conversations {
			conversationIDs = append(conversationIDs, conversation.Id)
		}

		var incidentIDs []int
		if err := tx.Model(&AssistantSecurityIncident{}).
			Where("conversation_id IN ?", conversationIDs).
			Pluck("id", &incidentIDs).Error; err != nil {
			return err
		}
		if len(incidentIDs) > 0 {
			if err := tx.Where("category = ? AND item_id IN ?", UnifiedTodoCategorySecurityIncident, incidentIDs).
				Delete(&UnifiedTodoRead{}).Error; err != nil {
				return err
			}
		}

		cards := tx.Where("conversation_id IN ?", conversationIDs).Delete(&AssistantSecureCard{})
		if cards.Error != nil {
			return cards.Error
		}
		messages := tx.Where("conversation_id IN ?", conversationIDs).Delete(&AssistantHistoryMessage{})
		if messages.Error != nil {
			return messages.Error
		}
		incidents := tx.Where("conversation_id IN ?", conversationIDs).Delete(&AssistantSecurityIncident{})
		if incidents.Error != nil {
			return incidents.Error
		}
		deleted := assistantRetentionEligible(
			tx.Where("id IN ?", conversationIDs),
			cutoffs,
		).Delete(&AssistantConversation{})
		if deleted.Error != nil {
			return deleted.Error
		}

		result.Conversations = deleted.RowsAffected
		result.Messages = messages.RowsAffected
		result.SecureCards = cards.RowsAffected
		result.Incidents = incidents.RowsAffected
		return nil
	})
	return result, err
}

// ScrubExpiredAssistantSecureCards erases ciphertext while retaining harmless
// card metadata for the transcript. It is bounded and safe to call repeatedly.
func ScrubExpiredAssistantSecureCards(ctx context.Context, now int64, batchSize int) (int64, error) {
	if now <= 0 {
		return 0, gorm.ErrInvalidData
	}
	batchSize = NormalizeAssistantRetentionBatchSize(batchSize)
	var ids []string
	if err := DB.WithContext(ctx).Model(&AssistantSecureCard{}).
		Where("ciphertext <> '' AND (expires_at <= ? OR revealed_at > 0)", now).
		Order("created_at ASC").Limit(batchSize).Pluck("id", &ids).Error; err != nil {
		return 0, err
	}
	if len(ids) == 0 {
		return 0, nil
	}
	updated := DB.WithContext(ctx).Model(&AssistantSecureCard{}).
		Where("id IN ? AND ciphertext <> '' AND (expires_at <= ? OR revealed_at > 0)", ids, now).
		Update("ciphertext", "")
	return updated.RowsAffected, updated.Error
}

// PurgeAdvancedSecurityEventsBefore removes old rule-match rows in bounded
// batches. Security events contain only digests and metadata, but they are
// produced per matched rule; without a retention boundary this audit table can
// grow with traffic forever.
func PurgeAdvancedSecurityEventsBefore(ctx context.Context, cutoff int64, batchSize int) (int64, error) {
	if cutoff <= 0 {
		return 0, gorm.ErrInvalidData
	}
	batchSize = NormalizeAssistantRetentionBatchSize(batchSize)
	var ids []uint
	if err := DB.WithContext(ctx).Model(&AdvancedSecurityEvent{}).
		Where("created_at < ?", cutoff).
		Order("created_at ASC, id ASC").Limit(batchSize).Pluck("id", &ids).Error; err != nil {
		return 0, err
	}
	if len(ids) == 0 {
		return 0, nil
	}
	deleted := DB.WithContext(ctx).Where("id IN ? AND created_at < ?", ids, cutoff).
		Delete(&AdvancedSecurityEvent{})
	return deleted.RowsAffected, deleted.Error
}

// PurgeAssistantIntentLeadsBefore removes old aggregate chat-intent rows in a
// bounded batch. Chat rows contain no transcript, but they still retain a user
// id and one row per uncached turn; retaining them forever would let routine
// assistant traffic grow the table without limit. Explicit support handoffs
// are excluded because they are an operator queue/history, not analytics.
func PurgeAssistantIntentLeadsBefore(ctx context.Context, cutoff int64, batchSize int) (int64, error) {
	if cutoff <= 0 {
		return 0, gorm.ErrInvalidData
	}
	batchSize = NormalizeAssistantRetentionBatchSize(batchSize)
	var ids []int
	if err := DB.WithContext(ctx).Model(&AssistantLead{}).
		Where("source = ? AND created_at < ?", AssistantLeadSourceChat, cutoff).
		Order("created_at ASC, id ASC").Limit(batchSize).Pluck("id", &ids).Error; err != nil {
		return 0, err
	}
	if len(ids) == 0 {
		return 0, nil
	}
	deleted := DB.WithContext(ctx).
		Where("id IN ? AND source = ? AND created_at < ?", ids, AssistantLeadSourceChat, cutoff).
		Delete(&AssistantLead{})
	return deleted.RowsAffected, deleted.Error
}

// PurgeAssistantUserProfileAuditsBefore removes old automatic profile-change
// audit rows in bounded batches. The audit stores only hashes and counts, but
// it is still one row per profile transition; source=administrator is kept as
// a durable operator audit and is not part of this assistant retention pass.
func PurgeAssistantUserProfileAuditsBefore(ctx context.Context, cutoff int64, batchSize int) (int64, error) {
	if cutoff <= 0 {
		return 0, gorm.ErrInvalidData
	}
	batchSize = NormalizeAssistantRetentionBatchSize(batchSize)
	var ids []int64
	if err := DB.WithContext(ctx).Model(&AssistantUserProfileAudit{}).
		Where("source = ? AND created_at < ?", AssistantProfileSourceAI, cutoff).
		Order("created_at ASC, id ASC").Limit(batchSize).Pluck("id", &ids).Error; err != nil {
		return 0, err
	}
	if len(ids) == 0 {
		return 0, nil
	}
	deleted := DB.WithContext(ctx).
		Where("id IN ? AND source = ? AND created_at < ?", ids, AssistantProfileSourceAI, cutoff).
		Delete(&AssistantUserProfileAudit{})
	return deleted.RowsAffected, deleted.Error
}
