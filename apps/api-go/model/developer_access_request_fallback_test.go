package model

import (
	"testing"

	"github.com/QuantumNous/new-api/common"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestAssistantL1RequestIsQueuedBeforeOptionalRecommendation(t *testing.T) {
	db := setupConsoleActivationTestDB(t)
	require.NoError(t, db.AutoMigrate(&DeveloperAccessRequest{}))

	user := User{
		Username: "fallback-queue-user",
		Password: "password",
		Role:     common.RoleCommonUser,
		Status:   common.UserStatusEnabled,
	}
	require.NoError(t, db.Create(&user).Error)

	queued, err := SubmitAssistantDeveloperAccessRequest(user.Id, "I need L1 access for a real integration.")
	require.NoError(t, err)
	assert.Equal(t, DeveloperAccessRequestPending, queued.Status)
	assert.Equal(t, DeveloperAccessRequestSourceAssistant, queued.Source)
	assert.Empty(t, queued.AIRecommendation)

	enriched, err := SubmitAssistantDeveloperAccessRecommendation(
		user.Id,
		"An edited statement replaces the earlier draft.",
		"The user described a concrete integration and can be reviewed for L1 access.",
	)
	require.NoError(t, err)
	assert.Equal(t, queued.Id, enriched.Id)
	assert.Equal(t, "An edited statement replaces the earlier draft.", enriched.Reason)
	assert.Equal(t, DeveloperAccessRequestSourceAI, enriched.Source)
	assert.NotEmpty(t, enriched.AIRecommendation)

	var stored DeveloperAccessRequest
	require.NoError(t, db.First(&stored, queued.Id).Error)
	assert.Equal(t, enriched.AIRecommendation, stored.AIRecommendation)
	assert.Equal(t, DeveloperAccessRequestPending, stored.Status)
}

func TestAssistantL1QueueEnrichmentAndApprovalIsOneDurableFlow(t *testing.T) {
	db := setupConsoleActivationTestDB(t)
	require.NoError(t, db.AutoMigrate(&DeveloperAccessRequest{}, &DeveloperAccessRecommendationArchive{}))

	user := User{
		Username: "assistant-l1-flow-user",
		Password: "password",
		Role:     common.RoleCommonUser,
		Status:   common.UserStatusEnabled,
	}
	require.NoError(t, db.Create(&user).Error)

	queued, err := SubmitAssistantDeveloperAccessRequest(user.Id, "I need L1 access for a real integration.")
	require.NoError(t, err)

	enriched, err := SubmitAssistantDeveloperAccessRecommendation(
		user.Id,
		"AI and human edits share one recommendation letter.",
		"The user described a concrete integration workflow and can be reviewed for L1 access.",
	)
	require.NoError(t, err)
	assert.Equal(t, queued.Id, enriched.Id)
	assert.Equal(t, "AI and human edits share one recommendation letter.", enriched.Reason)
	assert.Equal(t, DeveloperAccessRequestSourceAI, enriched.Source)

	var requestCount int64
	require.NoError(t, db.Model(&DeveloperAccessRequest{}).Where("user_id = ?", user.Id).Count(&requestCount).Error)
	assert.EqualValues(t, 1, requestCount)

	approved, err := ReviewDeveloperAccessRequest(99, enriched.Id, true, "approved for L1")
	require.NoError(t, err)
	assert.Equal(t, DeveloperAccessRequestApproved, approved.Status)

	var activated User
	require.NoError(t, db.First(&activated, user.Id).Error)
	assert.Positive(t, activated.ConsoleActivatedAt)
	access, err := GetDeveloperAccessStateForUserBase(activated.ToBaseUser())
	require.NoError(t, err)
	assert.True(t, access.Granted)
}

func TestDeveloperAccessRequestQueueFailureIsClassifiedForRetry(t *testing.T) {
	db := setupConsoleActivationTestDB(t)
	require.NoError(t, db.AutoMigrate(&DeveloperAccessRequest{}))
	user := User{
		Username: "queue-failure-classification-user",
		Password: "password",
		Role:     common.RoleCommonUser,
		Status:   common.UserStatusEnabled,
	}
	require.NoError(t, db.Create(&user).Error)
	require.NoError(t, db.Migrator().DropTable(&DeveloperAccessRequest{}))

	_, err := SubmitAssistantDeveloperAccessRequest(user.Id, "I need L1 access for a real integration.")
	assert.ErrorIs(t, err, ErrDeveloperAccessRequestQueueUnavailable)
}
