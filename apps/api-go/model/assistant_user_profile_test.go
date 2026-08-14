package model

import (
	"encoding/json"
	"fmt"
	"strings"
	"testing"

	"github.com/LIghtJUNction/api.lmm.best/common"
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

func TestAssistantUserProfileTagsRedactSecrets(t *testing.T) {
	tags, err := NormalizeAssistantProfileTags([]string{"api_key: sk-secret-value", "safe"})
	require.NoError(t, err)
	assert.NotContains(t, strings.Join(tags, ","), "sk-secret-value")
	assert.Contains(t, tags, "safe")
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

func TestPopulateAssistantUserProfilesUsesStrictLowerRoleVisibility(t *testing.T) {
	db := setupConsoleActivationTestDB(t)
	require.NoError(t, db.AutoMigrate(&AssistantUserProfile{}))
	viewer := &User{Username: "profile-viewer", Password: "password", Role: 100, AffCode: "profile-viewer"}
	target := &User{Username: "profile-target-visible", Password: "password", Role: 1, AffCode: "profile-target-visible"}
	peer := &User{Username: "profile-target-peer", Password: "password", Role: 10, AffCode: "profile-target-peer"}
	require.NoError(t, db.Create(viewer).Error)
	require.NoError(t, db.Create(target).Error)
	require.NoError(t, db.Create(peer).Error)
	_, err := SaveProfile(target.Id, target.Id, ProfileInput{
		Key: AssistantProfileTechnical, Tags: []string{"technical", "cost_sensitive"},
		Source: AssistantProfileSourceAI, Enabled: true,
	})
	require.NoError(t, err)
	_, err = SaveProfile(peer.Id, peer.Id, ProfileInput{
		Key: AssistantProfileGuided, Tags: []string{"needs_steps"},
		Source: AssistantProfileSourceAI, Enabled: true,
	})
	require.NoError(t, err)

	rows := []*User{viewer, target, peer}
	require.NoError(t, PopulateAssistantUserProfiles(rows, viewer.Id, viewer.Role))
	require.NotNil(t, target.AssistantProfile)
	assert.Equal(t, []string{"technical", "cost_sensitive"}, target.AssistantProfile.Tags)
	assert.Equal(t, AssistantProfileSourceAI, target.AssistantProfile.Source)
	assert.Nil(t, viewer.AssistantProfile)
	require.NotNil(t, peer.AssistantProfile)
	assert.Equal(t, []string{"needs_steps"}, peer.AssistantProfile.Tags)

	rows = []*User{target}
	require.NoError(t, PopulateAssistantUserProfiles(rows, target.Id, common.RoleCommonUser))
	assert.Nil(t, target.AssistantProfile)
}

func TestDeleteAssistantUserProfileOnlyAllowsOwnerToForgetAIProfile(t *testing.T) {
	db := setupConsoleActivationTestDB(t)
	require.NoError(t, db.AutoMigrate(&AssistantUserProfile{}))
	owner := newProfileOwner(t, db, "forget-owner")
	admin := newProfileOwner(t, db, "forget-admin")
	_, err := SaveProfile(owner.Id, owner.Id, ProfileInput{
		Key: AssistantProfileGuided, Tags: []string{"guided"}, Strategy: "short steps",
		Source: AssistantProfileSourceAI, Enabled: true,
	})
	require.NoError(t, err)

	assert.ErrorIs(t, DeleteAssistantUserProfile(owner.Id, admin.Id), ErrAssistantProfileOwner)
	assert.NoError(t, DeleteAssistantUserProfile(owner.Id, owner.Id))
	profile, err := GetAssistantUserProfile(owner.Id)
	require.NoError(t, err)
	assert.Nil(t, profile)
	assert.ErrorIs(t, DeleteAssistantUserProfile(owner.Id, owner.Id), ErrAssistantProfileMissing)
}

func TestDeleteAssistantUserProfileRefusesAdministratorOwnedProfile(t *testing.T) {
	db := setupConsoleActivationTestDB(t)
	require.NoError(t, db.AutoMigrate(&AssistantUserProfile{}))
	owner := newProfileOwner(t, db, "forget-managed")
	_, err := SaveProfile(owner.Id, 99, ProfileInput{
		Key: AssistantProfileOperator, Tags: []string{"production"}, Strategy: "admin",
		Source: AssistantProfileSourceAdmin, Enabled: true,
	})
	require.NoError(t, err)
	assert.ErrorIs(t, DeleteAssistantUserProfile(owner.Id, owner.Id), ErrAssistantProfileManaged)
	profile, err := GetAssistantUserProfile(owner.Id)
	require.NoError(t, err)
	assert.NotNil(t, profile)
}
