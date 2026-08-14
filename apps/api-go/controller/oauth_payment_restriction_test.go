package controller

import (
	"testing"

	"github.com/LIghtJUNction/api.lmm.best/model"
	"github.com/LIghtJUNction/api.lmm.best/oauth"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestApplyLinuxDOPaymentRestrictionRequiresScoreAboveThreshold(t *testing.T) {
	db := setupUserOnboardingTestDB(t)
	user := model.User{Username: "linuxdo-score-user", Password: "password123"}
	require.NoError(t, db.Create(&user).Error)
	provider := &oauth.LinuxDOProvider{}

	require.NoError(t, applyLinuxDOPaymentRestriction(provider, &oauth.OAuthUser{
		Extra: map[string]any{"gamification_score": float64(model.LinuxDOGamificationScorePaymentThreshold)},
	}, &user))
	assert.Zero(t, user.PaymentRestrictionFlags)
	require.NoError(t, db.First(&user, user.Id).Error)
	assert.Equal(t, float64(model.LinuxDOGamificationScorePaymentThreshold), user.LinuxDOGamificationScore)
	assert.Positive(t, user.LinuxDOScoreUpdatedAt)

	require.NoError(t, applyLinuxDOPaymentRestriction(provider, &oauth.OAuthUser{
		Extra: map[string]any{"gamification_score": float64(model.LinuxDOGamificationScorePaymentThreshold) + 0.5},
	}, &user))
	require.NoError(t, db.First(&user, user.Id).Error)
	assert.Equal(t, model.PaymentRestrictionLinuxDOHighScore, user.PaymentRestrictionFlags)

	require.NoError(t, applyLinuxDOPaymentRestriction(provider, &oauth.OAuthUser{
		Extra: map[string]any{"gamification_score": float64(50)},
	}, &user))
	require.NoError(t, db.First(&user, user.Id).Error)
	assert.Zero(t, user.PaymentRestrictionFlags)
	assert.Equal(t, float64(50), user.LinuxDOGamificationScore)
}

func TestApplyLinuxDOPaymentRestrictionIgnoresOtherProviders(t *testing.T) {
	db := setupUserOnboardingTestDB(t)
	user := model.User{Username: "github-score-user", Password: "password123"}
	require.NoError(t, db.Create(&user).Error)

	require.NoError(t, applyLinuxDOPaymentRestriction(&oauth.GitHubProvider{}, &oauth.OAuthUser{
		Extra: map[string]any{"gamification_score": float64(model.LinuxDOGamificationScorePaymentThreshold) + 1},
	}, &user))
	assert.Zero(t, user.PaymentRestrictionFlags)
}
