package model

import (
	"encoding/json"
	"fmt"
	"strings"
	"testing"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
	"gorm.io/gorm"
)

func newProfileOwner(t *testing.T, db *gorm.DB, suffix string) User {
	t.Helper()
	name := fmt.Sprintf("profile-%s", suffix)
	owner := User{Username: name, Password: "password", AffCode: name}
	require.NoError(t, db.Create(&owner).Error)
	return owner
}

func TestAssistantUserProfileNormalizesAndStoresInternalFields(t *testing.T) {
	db := setupConsoleActivationTestDB(t)
	require.NoError(t, db.AutoMigrate(&AssistantUserProfile{}))
	owner := newProfileOwner(t, db, "normalize")
	profile, err := UpsertAssistantUserProfile(owner.Id, 99, AssistantProfileGuided,
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
	owner := newProfileOwner(t, db, "empty")
	profile, err := UpsertAssistantUserProfile(owner.Id, 99, "", []string{"tag"}, "strategy", true)
	require.NoError(t, err)
	assert.False(t, profile.Enabled)
	assert.Positive(t, profile.UpdatedAt)
}

func TestAssistantProfileCannotOverwriteAdministratorStrategy(t *testing.T) {
	db := setupConsoleActivationTestDB(t)
	require.NoError(t, db.AutoMigrate(&AssistantUserProfile{}))
	owner := newProfileOwner(t, db, "admin-owned")

	admin, err := SaveProfile(owner.Id, 99, ProfileInput{
		Key: AssistantProfileOperator, Tags: []string{"production"}, Strategy: "Keep the administrator strategy.",
		Source: AssistantProfileSourceAdmin, Enabled: true,
	})
	require.NoError(t, err)
	_, err = SaveProfile(owner.Id, owner.Id, ProfileInput{
		Key: AssistantProfileGuided, Tags: []string{"needs_steps"}, Strategy: "Replace it from AI.",
		Source: AssistantProfileSourceAI, Enabled: true,
	})
	assert.ErrorIs(t, err, ErrAssistantProfileManaged)

	stored, err := GetAssistantUserProfile(owner.Id)
	require.NoError(t, err)
	assert.Equal(t, admin.ProfileKey, stored.ProfileKey)
	assert.Equal(t, admin.Strategy, stored.Strategy)
	assert.Equal(t, AssistantProfileSourceAdmin, stored.Source)
}

func TestAdministratorCanReplaceAssistantProfile(t *testing.T) {
	db := setupConsoleActivationTestDB(t)
	require.NoError(t, db.AutoMigrate(&AssistantUserProfile{}))
	owner := newProfileOwner(t, db, "ai-owned")

	_, err := SaveProfile(owner.Id, owner.Id, ProfileInput{
		Key: AssistantProfileGuided, Strategy: "AI strategy.", Source: AssistantProfileSourceAI, Enabled: true,
	})
	require.NoError(t, err)
	stored, err := SaveProfile(owner.Id, 99, ProfileInput{
		Key: AssistantProfileOperator, Strategy: "Administrator strategy.", Source: AssistantProfileSourceAdmin, Enabled: true,
	})
	require.NoError(t, err)
	assert.Equal(t, AssistantProfileOperator, stored.ProfileKey)
	assert.Equal(t, "Administrator strategy.", stored.Strategy)
	assert.Equal(t, AssistantProfileSourceAdmin, stored.Source)
}
