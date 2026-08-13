package model

import (
	"errors"
	"strings"
	"testing"

	"github.com/QuantumNous/new-api/common"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestAssistantDeveloperAccessRecommendationApprovalUnlocksL1WithoutPayment(t *testing.T) {
	db := setupConsoleActivationTestDB(t)
	require.NoError(t, db.AutoMigrate(&DeveloperAccessRequest{}))

	user := User{
		Username: "unlock-request-user",
		Password: "password",
		Role:     common.RoleCommonUser,
		Status:   common.UserStatusEnabled,
	}
	require.NoError(t, db.Create(&user).Error)

	request, err := SubmitAssistantDeveloperAccessRecommendation(
		user.Id,
		"I need access for a verified integration",
		"The user described a concrete integration and understands the API key setup steps.",
	)
	require.NoError(t, err)
	assert.Equal(t, DeveloperAccessRequestPending, request.Status)
	assert.Equal(t, DeveloperAccessRequestSourceAI, request.Source)
	assert.NotEmpty(t, request.AIRecommendation)

	// Repeated submissions edit the user's one pending recommendation letter.
	repeated, err := SubmitAssistantDeveloperAccessRecommendation(
		user.Id,
		"a second browser tab should not replace the original reason",
		"A second recommendation should not replace the original pending recommendation.",
	)
	require.NoError(t, err)
	assert.Equal(t, request.Id, repeated.Id)
	assert.Equal(t, "a second browser tab should not replace the original reason", repeated.Reason)
	assert.Equal(t, "A second recommendation should not replace the original pending recommendation.", repeated.AIRecommendation)
	assert.Equal(t, DeveloperAccessRequestSourceAI, repeated.Source)

	pending, err := ListDeveloperAccessRequests(DeveloperAccessRequestPending, 10)
	require.NoError(t, err)
	require.Len(t, pending, 1)
	assert.Equal(t, user.Username, pending[0].Username)

	approved, err := ReviewDeveloperAccessRequest(99, request.Id, true, "approved for L1")
	require.NoError(t, err)
	assert.Equal(t, DeveloperAccessRequestApproved, approved.Status)
	assert.Equal(t, 99, approved.AdminUserId)
	assert.Positive(t, approved.ReviewedAt)

	var activated User
	require.NoError(t, db.First(&activated, user.Id).Error)
	assert.Positive(t, activated.ConsoleActivatedAt)
	access, err := GetDeveloperAccessStateForUserBase(activated.ToBaseUser())
	require.NoError(t, err)
	assert.True(t, access.Granted)
	assert.False(t, access.PaidActivationComplete)

	_, err = ReviewDeveloperAccessRequest(99, request.Id, true, "duplicate")
	assert.ErrorIs(t, err, ErrDeveloperAccessRequestReviewed)
}

func TestAssistantDeveloperAccessRecommendationCanBeCleared(t *testing.T) {
	db := setupConsoleActivationTestDB(t)
	require.NoError(t, db.AutoMigrate(&DeveloperAccessRequest{}))
	user := User{Username: "clear-recommendation-user", Password: "password", Role: common.RoleCommonUser}
	require.NoError(t, db.Create(&user).Error)

	request, err := SubmitAssistantDeveloperAccessRecommendation(
		user.Id,
		"I use the relay for a concrete coding workflow.",
		"Recommend L1 because the user described a concrete coding workflow.",
	)
	require.NoError(t, err)
	require.NotEmpty(t, request.AIRecommendation)

	cleared, err := SubmitAssistantDeveloperAccessRequestWithoutRecommendation(user.Id, request.Reason)
	require.NoError(t, err)
	assert.Equal(t, request.Id, cleared.Id)
	assert.Empty(t, cleared.AIRecommendation)
	assert.Equal(t, DeveloperAccessRequestSourceAssistant, cleared.Source)
}

func TestAssistantDeveloperAccessRecommendationRejectionDoesNotActivate(t *testing.T) {
	db := setupConsoleActivationTestDB(t)
	require.NoError(t, db.AutoMigrate(&DeveloperAccessRequest{}))

	user := User{Username: "rejected-request-user", Password: "password", Role: common.RoleCommonUser}
	require.NoError(t, db.Create(&user).Error)
	request, err := SubmitAssistantDeveloperAccessRecommendation(
		user.Id,
		"please review my API integration",
		"The user has a plausible use case, but the administrator should request more detail.",
	)
	require.NoError(t, err)

	for _, note := range []string{"", " ", "a", "同"} {
		_, err := ReviewDeveloperAccessRequest(99, request.Id, false, note)
		assert.ErrorIs(t, err, ErrDeveloperAccessReviewNoteTooShort)
	}

	var stillPending DeveloperAccessRequest
	require.NoError(t, db.First(&stillPending, request.Id).Error)
	assert.Equal(t, DeveloperAccessRequestPending, stillPending.Status)

	rejected, err := ReviewDeveloperAccessRequest(99, request.Id, false, "please provide more detail")
	require.NoError(t, err)
	assert.Equal(t, DeveloperAccessRequestRejected, rejected.Status)

	var unchanged User
	require.NoError(t, db.First(&unchanged, user.Id).Error)
	assert.Zero(t, unchanged.ConsoleActivatedAt)
	latest, err := GetDeveloperAccessRequest(user.Id)
	require.NoError(t, err)
	require.NotNil(t, latest)
	assert.Equal(t, "please provide more detail", latest.AdminNote)
}

func TestDeveloperAccessRequestTextLimit(t *testing.T) {
	db := setupConsoleActivationTestDB(t)
	require.NoError(t, db.AutoMigrate(&DeveloperAccessRequest{}))
	user := User{Username: "long-request-user", Password: "password", Role: common.RoleCommonUser}
	require.NoError(t, db.Create(&user).Error)

	_, err := SubmitAssistantDeveloperAccessRecommendation(
		user.Id,
		string(make([]rune, maxDeveloperAccessRequestNote+1)),
		strings.Repeat("r", minDeveloperAccessRecommendation),
	)
	assert.ErrorIs(t, err, ErrDeveloperAccessRequestNoteTooLong)
	assert.False(t, errors.Is(err, ErrDeveloperAccessRequestReviewed))

	_, err = SubmitAssistantDeveloperAccessRecommendation(
		user.Id,
		"valid reason",
		string(make([]rune, maxDeveloperAccessRequestNote+1)),
	)
	assert.ErrorIs(t, err, ErrDeveloperAccessRequestNoteTooLong)
}

func TestAssistantDeveloperAccessRecommendationValidationAndRedaction(t *testing.T) {
	db := setupConsoleActivationTestDB(t)
	require.NoError(t, db.AutoMigrate(&DeveloperAccessRequest{}))
	user := User{Username: "short-request-user", Password: "password", Role: common.RoleCommonUser}
	require.NoError(t, db.Create(&user).Error)

	for _, reason := range []string{"", "   ", "abcd", "  四个字  "} {
		_, err := SubmitAssistantDeveloperAccessRecommendation(
			user.Id,
			reason,
			strings.Repeat("r", minDeveloperAccessRecommendation),
		)
		assert.ErrorIs(t, err, ErrDeveloperAccessRequestReasonTooShort)
	}

	for _, recommendation := range []string{"", "   ", strings.Repeat("推", minDeveloperAccessRecommendation-1)} {
		_, err := SubmitAssistantDeveloperAccessRecommendation(user.Id, "valid reason", recommendation)
		assert.ErrorIs(t, err, ErrDeveloperAccessRecommendationTooShort)
	}

	request, err := SubmitAssistantDeveloperAccessRecommendation(
		user.Id,
		"  测试申请说，password: hunter2  ",
		"AI recommends approval because the use case is clear; key=sk-secret-token-123.",
	)
	require.NoError(t, err)
	assert.Equal(t, DeveloperAccessRequestSourceAI, request.Source)
	assert.NotContains(t, request.Reason, "hunter2")
	assert.NotContains(t, request.AIRecommendation, "sk-secret-token-123")
	assert.Contains(t, request.Reason, "[REDACTED]")
	assert.Contains(t, request.AIRecommendation, "[REDACTED_API_KEY]")

	var persisted DeveloperAccessRequest
	require.NoError(t, db.First(&persisted, request.Id).Error)
	assert.Equal(t, request.Reason, persisted.Reason)
	assert.Equal(t, request.AIRecommendation, persisted.AIRecommendation)
	assert.Equal(t, DeveloperAccessRequestSourceAI, persisted.Source)
}
