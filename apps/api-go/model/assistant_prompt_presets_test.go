package model

import (
	"testing"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func setupPromptPresetTestDB(t *testing.T) {
	t.Helper()
	_ = setupAssistantLeadTestDB(t)
	require.NoError(t, DB.AutoMigrate(
		&PromptPresetRow{},
		&PromptPresetStat{},
		&PromptConversionRef{},
		&PromptConversationRef{},
	))
}

func TestPromptPresetFallbackAndAggregateAttribution(t *testing.T) {
	setupPromptPresetTestDB(t)

	set, err := GetPromptPresets()
	require.NoError(t, err)
	assert.EqualValues(t, 0, set.Generation)
	assert.Equal(t, fallbackPresetVersion, set.Version)
	require.Len(t, set.Presets, maxPromptPresets)
	assert.Equal(t, "getting_started", set.Presets[0].Id)
	assert.NotEmpty(t, set.Presets[0].Prompt)

	attribution, err := ResolvePromptPreset(set.Presets[0].Id, set.Presets[0].Prompt)
	require.NoError(t, err)
	require.NoError(t, CountPresetClick(set.Presets[0].Id))
	require.NoError(t, CountPresetClick(set.Presets[0].Id))
	require.NoError(t, CountPresetConversation(*attribution, 37))
	conversationAttribution, err := ConversationPreset(37)
	require.NoError(t, err)
	assert.Equal(t, attribution, conversationAttribution)
	require.NoError(t, CountPresetRecommendation(*attribution, 71))
	require.NoError(t, CountPresetApproval(71))

	var stat PromptPresetStat
	require.NoError(t, DB.Where("preset_id = ?", set.Presets[0].Id).First(&stat).Error)
	assert.EqualValues(t, 2, stat.ClickCount)
	assert.EqualValues(t, 1, stat.ConversationCount)
	assert.EqualValues(t, 1, stat.RecommendationCount)
	assert.EqualValues(t, 1, stat.ApprovalCount)

	assert.False(t, DB.Migrator().HasColumn(&PromptPresetStat{}, "user_id"))
	assert.False(t, DB.Migrator().HasColumn(&PromptConversionRef{}, "user_id"))
	assert.False(t, DB.Migrator().HasColumn(&PromptConversionRef{}, "message"))
	assert.False(t, DB.Migrator().HasColumn(&PromptConversationRef{}, "user_id"))
	var attributionCount int64
	require.NoError(t, DB.Model(&PromptConversionRef{}).Count(&attributionCount).Error)
	assert.Zero(t, attributionCount)
}

func TestPromptPresetValidationAndBoundedRefresh(t *testing.T) {
	setupPromptPresetTestDB(t)

	_, err := ResolvePromptPreset("getting_started", "client supplied replacement")
	assert.ErrorIs(t, err, ErrPromptPresetNotFound)
	assert.ErrorIs(t, CountPresetClick("unknown"), ErrPromptPresetNotFound)

	for range 3 {
		require.NoError(t, RecordAssistantFirstQuestion("请帮我估算模型 token 的费用和价格；邮箱 alice@example.com；key=sk-secret-token-123"))
	}
	generated, err := RefreshPromptPresets()
	require.NoError(t, err)
	assert.Positive(t, generated.Generation)
	require.NotEmpty(t, generated.Presets)
	assert.LessOrEqual(t, len(generated.Presets), maxPromptPresets)
	assert.Equal(t, "pricing_cost", generated.Presets[0].Id)
	assert.NotEqual(t, promptCandidates[3].Prompt, generated.Presets[0].Prompt)
	assert.Contains(t, generated.Presets[0].Prompt, "费用")
	assert.NotContains(t, generated.Presets[0].Prompt, "alice")
	assert.NotContains(t, generated.Presets[0].Prompt, "secret")

	for range presetGenerations + 2 {
		_, err = RefreshPromptPresets()
		require.NoError(t, err)
	}
	var generations []int64
	require.NoError(t, DB.Model(&PromptPresetRow{}).
		Distinct("generation").Pluck("generation", &generations).Error)
	assert.LessOrEqual(t, len(generations), presetGenerations)
	var rows int64
	require.NoError(t, DB.Model(&PromptPresetRow{}).Count(&rows).Error)
	assert.LessOrEqual(t, rows, int64(presetGenerations*maxPromptPresets))
}
