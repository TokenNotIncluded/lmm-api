package model

import (
	"encoding/json"
	"strings"
	"testing"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestAssistantUserProfileNormalizesAndStoresInternalFields(t *testing.T) {
	db := setupConsoleActivationTestDB(t)
	require.NoError(t, db.AutoMigrate(&AssistantUserProfile{}))
	userID := 41
	profile, err := UpsertAssistantUserProfile(userID, 99, AssistantProfileGuided,
		[]string{"guided", " guided ", "needs-help"},
		"Ask one focused question. api_key: sk-secret-value; never expose this note.", true)
	require.NoError(t, err)
	assert.True(t, profile.Enabled)
	assert.Equal(t, "Ask one focused question. api_key: [REDACTED]; never expose this note.", profile.Strategy)
	assert.Equal(t, []string{"guided", "needs-help"}, AssistantUserProfileTags(profile))
	view := AssistantUserProfileViewOf(profile)
	encoded, err := json.Marshal(view)
	require.NoError(t, err)
	assert.Contains(t, string(encoded), "needs-help")
	assert.NotContains(t, string(encoded), "sk-secret-value")
	internalEncoded, err := json.Marshal(profile)
	require.NoError(t, err)
	assert.Equal(t, "{}", string(internalEncoded))
}

func TestAssistantUserProfileRejectsUnsafeOrOversizedValues(t *testing.T) {
	_, err := NormalizeAssistantProfileKey("unknown-skill")
	assert.ErrorIs(t, err, ErrAssistantProfileKey)
	_, err = NormalizeAssistantProfileTags([]string{strings.Repeat("x", AssistantUserProfileMaxTagRunes+1)})
	assert.ErrorIs(t, err, ErrAssistantProfileTagsInvalid)
	_, err = NormalizeAssistantProfileTags(make([]string, AssistantUserProfileMaxTags+1))
	assert.ErrorIs(t, err, ErrAssistantProfileTagsInvalid)
	_, err = NormalizeAssistantProfileStrategy(strings.Repeat("策略 ", AssistantUserProfileMaxStrategyRunes))
	assert.ErrorIs(t, err, ErrAssistantProfileStrategyLong)
}

func TestAssistantUserProfileEmptyKeyAlwaysDisables(t *testing.T) {
	db := setupConsoleActivationTestDB(t)
	require.NoError(t, db.AutoMigrate(&AssistantUserProfile{}))
	profile, err := UpsertAssistantUserProfile(42, 99, "", []string{"tag"}, "strategy", true)
	require.NoError(t, err)
	assert.False(t, profile.Enabled)
	assert.Positive(t, profile.UpdatedAt)
}
