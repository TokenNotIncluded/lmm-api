package model

import (
	"strconv"
	"testing"

	"github.com/stretchr/testify/require"
)

func TestPruneWaffoPancakeWebhookReceiptsIsBoundedAndKeepsRecentRows(t *testing.T) {
	db := setupConsoleActivationTestDB(t)
	require.NoError(t, db.AutoMigrate(&WaffoPancakeWebhookReceipt{}))
	t.Setenv("WAFFO_PANCAKE_WEBHOOK_RECEIPT_RETENTION_SECONDS", "2700")

	now := int64(1_000_000)
	receipts := make([]WaffoPancakeWebhookReceipt, 0, 258)
	for i := 0; i < 257; i++ {
		receipts = append(receipts, WaffoPancakeWebhookReceipt{
			Provider:   "waffo_pancake",
			EventID:    "old-" + strconv.Itoa(i),
			EventType:  "refund.failed",
			ReceivedAt: now - 2701,
		})
	}
	receipts = append(receipts, WaffoPancakeWebhookReceipt{
		Provider:   "waffo_pancake",
		EventID:    "recent",
		EventType:  "refund.failed",
		ReceivedAt: now - 1,
	})
	require.NoError(t, db.Create(&receipts).Error)

	deleted, err := PruneWaffoPancakeWebhookReceipts(now)
	require.NoError(t, err)
	require.EqualValues(t, 256, deleted)
	var remaining []WaffoPancakeWebhookReceipt
	require.NoError(t, db.Find(&remaining).Error)
	require.Len(t, remaining, 2)
	var recent WaffoPancakeWebhookReceipt
	require.NoError(t, db.Where("event_id = ?", "recent").First(&recent).Error)
}
