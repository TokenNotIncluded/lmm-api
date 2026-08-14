package controller

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"
	"time"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/LIghtJUNction/api.lmm.best/model"
	"github.com/LIghtJUNction/api.lmm.best/setting/operation_setting"
	"github.com/gin-gonic/gin"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func withPaymentMethods(t *testing.T, methods []map[string]string) {
	t.Helper()
	original := operation_setting.PayMethods
	operation_setting.PayMethods = methods
	t.Cleanup(func() { operation_setting.PayMethods = original })
}

func TestPaymentMethodUnlockDelayUsesFullDaysAfterRegistration(t *testing.T) {
	now := time.Unix(2_000_000_000, 0)
	withPaymentMethods(t, []map[string]string{{
		"name": "Card", "type": "stripe", "unlock_after_days": "2", "audience_mode": "all",
	}})

	user := &model.User{CreatedAt: now.Add(-48*time.Hour + time.Second).Unix()}
	available, unlockAt, err := paymentMethodAvailableForUser(user, "stripe", now)
	require.NoError(t, err)
	assert.False(t, available)
	assert.Equal(t, user.CreatedAt+2*secondsPerDay, unlockAt)

	user.CreatedAt = now.Add(-48 * time.Hour).Unix()
	available, _, err = paymentMethodAvailableForUser(user, "stripe", now)
	require.NoError(t, err)
	assert.True(t, available)
}

func TestPaymentMethodAudiencePreservesLegacyMarkerUntilExplicitlyConfigured(t *testing.T) {
	user := &model.User{PaymentRestrictionFlags: model.PaymentRestrictionLinuxDOHighScore}
	withPaymentMethods(t, []map[string]string{{"name": "Card", "type": "stripe"}})
	visible, err := paymentMethodVisibleForUser(user, "stripe")
	require.NoError(t, err)
	assert.False(t, visible)

	operation_setting.PayMethods[0]["audience_mode"] = "all"
	visible, err = paymentMethodVisibleForUser(user, "stripe")
	require.NoError(t, err)
	assert.True(t, visible)
}

func TestPaymentMethodAudienceMatchesEmailOAuthAndLinuxDOScore(t *testing.T) {
	user := &model.User{
		Email:                    "member@mail.linux.do",
		LinuxDOId:                "42",
		LinuxDOGamificationScore: 12_500.5,
		LinuxDOScoreUpdatedAt:    1,
		PaymentRestrictionFlags:  model.PaymentRestrictionLinuxDOHighScore,
	}

	t.Run("email contains preset", func(t *testing.T) {
		withPaymentMethods(t, []map[string]string{{
			"name": "Card", "type": "stripe", "audience_mode": "include",
			"audience_email_contains": "linux.do",
		}})
		visible, err := paymentMethodVisibleForUser(user, "stripe")
		require.NoError(t, err)
		assert.True(t, visible)
	})

	t.Run("OAuth exclusion", func(t *testing.T) {
		withPaymentMethods(t, []map[string]string{{
			"name": "Card", "type": "stripe", "audience_mode": "exclude",
			"audience_oauth_provider": "linux.do",
		}})
		visible, err := paymentMethodVisibleForUser(user, "stripe")
		require.NoError(t, err)
		assert.False(t, visible)
	})

	t.Run("score range", func(t *testing.T) {
		withPaymentMethods(t, []map[string]string{{
			"name": "Card", "type": "stripe", "audience_mode": "include",
			"audience_linuxdo_score_min": "10000", "audience_linuxdo_score_max": "13000",
		}})
		visible, err := paymentMethodVisibleForUser(user, "stripe")
		require.NoError(t, err)
		assert.True(t, visible)

		operation_setting.PayMethods[0]["audience_linuxdo_score_max"] = "12000"
		visible, err = paymentMethodVisibleForUser(user, "stripe")
		require.NoError(t, err)
		assert.False(t, visible)
	})
}

func TestPaymentMethodAudienceSupportsAnyAndAllConditionMatching(t *testing.T) {
	user := &model.User{Email: "member@example.com", LinuxDOId: "42"}
	method := map[string]string{
		"name": "Card", "type": "stripe", "audience_mode": "include",
		"audience_email_contains": "linux.do", "audience_oauth_provider": "linuxdo",
	}
	withPaymentMethods(t, []map[string]string{method})

	visible, err := paymentMethodVisibleForUser(user, "stripe")
	require.NoError(t, err)
	assert.True(t, visible, "any is the default")

	method["audience_match"] = "all"
	visible, err = paymentMethodVisibleForUser(user, "stripe")
	require.NoError(t, err)
	assert.False(t, visible)
}

func TestPaymentMethodAudienceMatchesUserGroupAndRole(t *testing.T) {
	user := &model.User{Group: "vip", Role: common.RoleAdminUser}
	method := map[string]string{
		"name": "Card", "type": "stripe", "audience_mode": "include",
		"audience_match": "all", "audience_user_group": "default, vip",
		"audience_role": "admin",
	}
	withPaymentMethods(t, []map[string]string{method})

	visible, err := paymentMethodVisibleForUser(user, "stripe")
	require.NoError(t, err)
	assert.True(t, visible)

	user.Group = "default"
	visible, err = paymentMethodVisibleForUser(user, "stripe")
	require.NoError(t, err)
	assert.True(t, visible)

	user.Role = common.RoleCommonUser
	visible, err = paymentMethodVisibleForUser(user, "stripe")
	require.NoError(t, err)
	assert.False(t, visible)
}

func TestDisabledPaymentMethodIsUnavailableAndInvalidEnabledFailsClosed(t *testing.T) {
	user := &model.User{}
	withPaymentMethods(t, []map[string]string{{
		"name": "Card", "type": "stripe", "enabled": "false",
	}})
	available, _, err := paymentMethodAvailableForUser(user, "stripe", time.Now())
	require.NoError(t, err)
	assert.False(t, available)

	operation_setting.PayMethods[0]["enabled"] = "invalid"
	available, _, err = paymentMethodAvailableForUser(user, "stripe", time.Now())
	assert.Error(t, err)
	assert.False(t, available)
}

func TestSanitizedPaymentMethodsHideServerOnlyPolicyFields(t *testing.T) {
	methods := []map[string]string{{
		"name":                    "Card",
		"type":                    "stripe",
		"enabled":                 "false",
		"description":             "Scheduled maintenance",
		"color":                   "#123456",
		"audience_mode":           "exclude",
		"audience_user_group":     "vip",
		"audience_role":           "admin",
		"audience_email_contains": "linux.do",
	}}

	public := sanitizedPaymentMethods(methods)
	require.Len(t, public, 1)
	assert.Equal(t, "Scheduled maintenance", public[0]["description"])
	assert.Equal(t, "#123456", public[0]["color"])
	_, hasEnabled := public[0]["enabled"]
	_, hasAudience := public[0]["audience_mode"]
	assert.False(t, hasEnabled)
	assert.False(t, hasAudience)
}

func TestPaymentMethodAudienceRejectsInvalidOrEmptyRules(t *testing.T) {
	user := &model.User{}
	for _, method := range []map[string]string{
		{"name": "Card", "type": "stripe", "audience_mode": "include"},
		{"name": "Card", "type": "stripe", "audience_mode": "exclude", "audience_oauth_provider": "unknown"},
		{"name": "Card", "type": "stripe", "audience_mode": "include", "audience_linuxdo_score_min": "20", "audience_linuxdo_score_max": "10"},
		{"name": "Card", "type": "stripe", "unlock_after_days": "-1"},
	} {
		withPaymentMethods(t, []map[string]string{method})
		available, _, err := paymentMethodAvailableForUser(user, "stripe", time.Now())
		assert.Error(t, err)
		assert.False(t, available)
	}
}

func TestGetTopUpInfoFiltersMethodUntilRegistrationDelayExpires(t *testing.T) {
	gin.SetMode(gin.TestMode)
	confirmPaymentComplianceForTest(t)
	preservePaymentGatewaySettings(t)
	setupTopupInfoUser(t, 901, "default")
	operation_setting.PayAddress = "https://epay.example.com"
	operation_setting.EpayId = "merchant"
	operation_setting.EpayKey = "secret"
	operation_setting.PayMethods = []map[string]string{{
		"name": "Delayed card", "type": "alipay", "unlock_after_days": "2", "audience_mode": "all",
	}}

	getInfo := func() struct {
		Data struct {
			EnableOnlineTopUp bool                `json:"enable_online_topup"`
			PayMethods        []map[string]string `json:"pay_methods"`
		} `json:"data"`
	} {
		response := httptest.NewRecorder()
		context, _ := gin.CreateTestContext(response)
		context.Set("id", 901)
		context.Request = httptest.NewRequest(http.MethodGet, "/api/user/topup/info", nil)
		GetTopUpInfo(context)

		var payload struct {
			Data struct {
				EnableOnlineTopUp bool                `json:"enable_online_topup"`
				PayMethods        []map[string]string `json:"pay_methods"`
			} `json:"data"`
		}
		require.NoError(t, json.Unmarshal(response.Body.Bytes(), &payload))
		return payload
	}

	locked := getInfo()
	assert.False(t, locked.Data.EnableOnlineTopUp)
	assert.Empty(t, locked.Data.PayMethods)

	require.NoError(t, model.DB.Model(&model.User{}).Where("id = ?", 901).
		Update("created_at", time.Now().Add(-72*time.Hour).Unix()).Error)
	unlocked := getInfo()
	assert.True(t, unlocked.Data.EnableOnlineTopUp)
	require.Len(t, unlocked.Data.PayMethods, 1)
	assert.Equal(t, "alipay", unlocked.Data.PayMethods[0]["type"])
}
