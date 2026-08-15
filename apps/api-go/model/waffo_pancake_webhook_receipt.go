package model

import (
	"strings"
	"time"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"gorm.io/gorm"
)

const (
	// Pancake's SDK rejects signatures older than 45 minutes by default. Keep
	// receipts longer than that window so a valid delayed retry cannot create a
	// second audit log, while still bounding this idempotency table.
	waffoPancakeWebhookReceiptDefaultRetentionSeconds = 48 * 60 * 60
	waffoPancakeWebhookReceiptMinimumRetentionSeconds = 45 * 60
	waffoPancakeWebhookReceiptCleanupBatchSize        = 256
)

// WaffoPancakeWebhookReceipt claims a provider event before a non-ledger
// side-effect is emitted. Finance ledger rows already have their own
// idempotency key; refund.failed only creates an audit log, so it needs a
// durable receipt of its own to make retries safe across API processes.
type WaffoPancakeWebhookReceipt struct {
	ID         int64  `json:"id" gorm:"primaryKey"`
	Provider   string `json:"provider" gorm:"type:varchar(64);not null;uniqueIndex:idx_waffo_pancake_webhook_receipt,priority:1"`
	EventID    string `json:"event_id" gorm:"type:varchar(180);not null;uniqueIndex:idx_waffo_pancake_webhook_receipt,priority:2"`
	EventType  string `json:"event_type" gorm:"type:varchar(64);not null;index"`
	ReceivedAt int64  `json:"received_at" gorm:"not null;index"`
}

// ClaimWaffoPancakeWebhookEvent atomically claims one provider event. A
// duplicate unique-key insert is a normal replay and is reported as
// claimed=false; other database errors are returned so the provider retries
// safely.
func ClaimWaffoPancakeWebhookEvent(provider, eventID, eventType string) (claimed bool, err error) {
	provider = strings.TrimSpace(provider)
	eventID = strings.TrimSpace(eventID)
	eventType = strings.TrimSpace(eventType)
	if DB == nil || provider == "" || eventID == "" || eventType == "" {
		return false, gorm.ErrInvalidData
	}
	receipt := &WaffoPancakeWebhookReceipt{
		Provider:   provider,
		EventID:    eventID,
		EventType:  eventType,
		ReceivedAt: time.Now().Unix(),
	}
	if err := DB.Create(receipt).Error; err != nil {
		if uniqueConstraintError(err) {
			return false, nil
		}
		return false, err
	}
	// Cleanup is best effort: a temporary cleanup failure must not turn an
	// already accepted, signed provider event into a retry storm.
	_, _ = PruneWaffoPancakeWebhookReceipts(receipt.ReceivedAt)
	return true, nil
}

func waffoPancakeWebhookReceiptRetentionSeconds() int64 {
	retention := common.GetEnvOrDefault(
		"WAFFO_PANCAKE_WEBHOOK_RECEIPT_RETENTION_SECONDS",
		waffoPancakeWebhookReceiptDefaultRetentionSeconds,
	)
	if retention < waffoPancakeWebhookReceiptMinimumRetentionSeconds {
		retention = waffoPancakeWebhookReceiptMinimumRetentionSeconds
	}
	return int64(retention)
}

// PruneWaffoPancakeWebhookReceipts removes at most one bounded batch of
// receipts older than the SDK's replay window. The corresponding user audit
// log remains intact; only the replay guard is pruned.
func PruneWaffoPancakeWebhookReceipts(now int64) (int64, error) {
	if DB == nil {
		return 0, gorm.ErrInvalidData
	}
	if now <= 0 {
		now = time.Now().Unix()
	}
	cutoff := now - waffoPancakeWebhookReceiptRetentionSeconds()
	var ids []int64
	if err := DB.Model(&WaffoPancakeWebhookReceipt{}).
		Where("received_at < ?", cutoff).
		Order("received_at ASC").
		Limit(waffoPancakeWebhookReceiptCleanupBatchSize).
		Pluck("id", &ids).Error; err != nil {
		return 0, err
	}
	if len(ids) == 0 {
		return 0, nil
	}
	result := DB.Delete(&WaffoPancakeWebhookReceipt{}, ids)
	return result.RowsAffected, result.Error
}
