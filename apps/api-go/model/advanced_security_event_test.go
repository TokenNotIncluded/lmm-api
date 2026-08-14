package model

import (
	"context"
	"testing"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestAdvancedSecurityEventsPersistWithoutPromptTextAndAggregate(t *testing.T) {
	db := setupConsoleActivationTestDB(t)
	require.NoError(t, db.AutoMigrate(&AdvancedSecurityEvent{}))

	require.NoError(t, RecordAdvancedSecurityEvents(context.Background(), AdvancedSecurityEventParams{
		CreatedAt:   100,
		RequestID:   "request-1",
		UserID:      7,
		Username:    "alice",
		ModelName:   "claude-test",
		Decision:    AdvancedSecurityDecisionBlocked,
		InputDigest: "input-digest",
		Matches: []AdvancedSecurityEventMatch{
			{RuleID: "violence", RuleName: "Violence", Category: "violence", Layer: "universal_standard", Severity: "high", PatternDigest: "pattern-digest"},
			{RuleID: "self-harm", RuleName: "Self harm", Category: "self_harm", Severity: "high", PatternDigest: "another-digest"},
		},
	}))
	require.NoError(t, RecordAdvancedSecurityEvents(context.Background(), AdvancedSecurityEventParams{
		CreatedAt: 200,
		RequestID: "request-2",
		UserID:    8,
		Username:  "bob",
		Decision:  AdvancedSecurityDecisionAudited,
		Matches: []AdvancedSecurityEventMatch{
			{RuleID: "violence", RuleName: "Violence", Category: "violence", Severity: "high"},
		},
	}))

	var stored AdvancedSecurityEvent
	require.NoError(t, db.Order("id asc").First(&stored).Error)
	assert.Equal(t, "input-digest", stored.InputDigest)
	assert.Equal(t, "pattern-digest", stored.PatternDigest)
	assert.Equal(t, "universal_standard", stored.Layer)
	assert.NotContains(t, stored.InputDigest, "prompt")

	stats, err := GetAdvancedSecurityStats(AdvancedSecurityEventFilter{})
	require.NoError(t, err)
	assert.EqualValues(t, 3, stats.TotalMatches)
	assert.EqualValues(t, 2, stats.BlockedMatches)
	assert.EqualValues(t, 1, stats.AuditedMatches)
	assert.EqualValues(t, 2, stats.AffectedRequests)
	assert.EqualValues(t, 2, stats.AffectedUsers)
	assert.Equal(t, []AdvancedSecurityStatBucket{
		{Key: "violence", Count: 2},
		{Key: "self_harm", Count: 1},
	}, stats.ByCategory)

	events, total, err := ListAdvancedSecurityEvents(AdvancedSecurityEventFilter{
		Decision: AdvancedSecurityDecisionBlocked,
		Limit:    1,
	})
	require.NoError(t, err)
	assert.EqualValues(t, 2, total)
	require.Len(t, events, 1)
	assert.Equal(t, "self-harm", events[0].RuleID)
}

func TestRecordAdvancedSecurityEventsRejectsInvalidDecision(t *testing.T) {
	db := setupConsoleActivationTestDB(t)
	require.NoError(t, db.AutoMigrate(&AdvancedSecurityEvent{}))
	err := RecordAdvancedSecurityEvents(context.Background(), AdvancedSecurityEventParams{
		Decision: "unknown",
		Matches:  []AdvancedSecurityEventMatch{{RuleID: "test", Category: "custom"}},
	})
	assert.Error(t, err)
}

func TestPurgeAdvancedSecurityEventsBeforeUsesBoundedBatches(t *testing.T) {
	db := setupConsoleActivationTestDB(t)
	require.NoError(t, db.AutoMigrate(&AdvancedSecurityEvent{}))
	rows := make([]AdvancedSecurityEvent, 0, 401)
	for index := 0; index < 401; index++ {
		rows = append(rows, AdvancedSecurityEvent{
			CreatedAt:   int64(index + 1),
			RequestID:   "request",
			RuleID:      "rule",
			Category:    "category",
			Decision:    AdvancedSecurityDecisionAudited,
			InputDigest: "digest",
		})
	}
	require.NoError(t, db.Create(&rows).Error)

	var removed int64
	for {
		batch, err := PurgeAdvancedSecurityEventsBefore(context.Background(), 402, 200)
		require.NoError(t, err)
		if batch == 0 {
			break
		}
		removed += batch
	}
	assert.EqualValues(t, 401, removed)
	var remaining int64
	require.NoError(t, db.Model(&AdvancedSecurityEvent{}).Count(&remaining).Error)
	assert.Zero(t, remaining)
}
