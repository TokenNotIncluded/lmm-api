package controller

import (
	"net/http"
	"net/http/httptest"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/LIghtJUNction/api.lmm.best/model"
	"github.com/LIghtJUNction/api.lmm.best/setting"
	"github.com/gin-gonic/gin"
	"github.com/stretchr/testify/require"
)

func TestAssistantReviewPolicyUsesGroupOverrideAndDefaultOff(t *testing.T) {
	original := setting.GetAssistantSettings()
	t.Cleanup(func() {
		setting.SetAssistantReviewEnabled(original.ReviewEnabled)
		_ = setting.UpdateAssistantReviewProbability(formatReviewProbability(original.ReviewProbability))
		_ = setting.UpdateAssistantReviewModel(original.ReviewModel)
		_ = setting.UpdateAssistantReviewGroupPolicies(setting.AssistantReviewGroupPoliciesJSON(original.ReviewGroupPolicies))
	})
	setting.SetAssistantReviewEnabled(true)
	require.NoError(t, setting.UpdateAssistantReviewProbability("0"))
	require.NoError(t, setting.UpdateAssistantReviewGroupPolicies(`{"default":{"probability":1,"intensity":"high"}}`))
	settings := setting.GetAssistantSettings()
	probability, intensity, enabled := assistantReviewPolicy(settings, "default")
	require.True(t, enabled)
	require.Equal(t, 1.0, probability)
	require.Equal(t, "high", intensity)
	_, _, enabled = assistantReviewPolicy(settings, "other")
	require.False(t, enabled)
}

func TestParseAssistantReviewDecisionDropsExplanationForNegative(t *testing.T) {
	decision, err := parseAssistantReviewDecision([]byte(`{"violation":false,"abuse":true,"rules":["x"],"explanation":"secret"}`))
	require.NoError(t, err)
	require.False(t, decision.Violation)
	require.False(t, decision.Abuse)
	require.Empty(t, decision.Rules)
	require.Empty(t, decision.Explanation)

	decision, err = parseAssistantReviewDecision([]byte(`prefix {"violation":true,"abuse":true,"rules":["security bypass"],"explanation":"attempted evasion"} suffix`))
	require.NoError(t, err)
	require.True(t, decision.Violation)
	require.True(t, decision.Abuse)
	require.Equal(t, []string{"security bypass"}, decision.Rules)
	require.Equal(t, "attempted evasion", decision.Explanation)
}

func TestAssistantReviewConversationKeepsLatestTurnWithinBudget(t *testing.T) {
	latest := "please inspect this latest request"
	messages := []assistantOpenAIMessage{
		{Role: "user", Content: strings.Repeat("old context ", 2_000)},
		{Role: "assistant", Content: strings.Repeat("old answer ", 2_000)},
		{Role: "user", Content: latest},
	}

	bounded := assistantReviewConversation(messages)
	require.NotEmpty(t, bounded)
	require.Equal(t, "user", bounded[len(bounded)-1].Role)
	require.Equal(t, latest, bounded[len(bounded)-1].Content)
}

func TestAssistantReviewQueueDropIsCountedWithoutBlocking(t *testing.T) {
	previousQueue := assistantReviewQueue.Load()
	previousEnqueued := assistantReviewQueueEnqueued.Load()
	previousDropped := assistantReviewQueueDropped.Load()
	previousAlertAt := assistantReviewDropAlertAt.Load()
	t.Cleanup(func() {
		assistantReviewQueue.Store(previousQueue)
		assistantReviewQueueEnqueued.Store(previousEnqueued)
		assistantReviewQueueDropped.Store(previousDropped)
		assistantReviewDropAlertAt.Store(previousAlertAt)
	})

	queue := make(chan assistantRequestReviewJob, 1)
	assistantReviewQueue.Store(&queue)
	assistantReviewQueueEnqueued.Store(0)
	assistantReviewQueueDropped.Store(0)
	// Keep this unit test quiet; the production path rate-limits the alert.
	assistantReviewDropAlertAt.Store(time.Now().UnixNano())

	job := assistantRequestReviewJob{UserID: 42, Model: "review-model"}
	require.True(t, offerAssistantReviewJob(job))
	started := time.Now()
	require.False(t, offerAssistantReviewJob(job))
	require.Less(t, time.Since(started), 100*time.Millisecond)

	stats := assistantReviewQueueStatsSnapshot()
	require.Equal(t, 1, stats.Depth)
	require.EqualValues(t, 1, stats.Enqueued)
	require.EqualValues(t, 1, stats.Dropped)
}

func formatReviewProbability(probability float64) string {
	return strconv.FormatFloat(probability, 'f', -1, 64)
}

func TestAdminAssistantRequestReviewsRespectRoleScope(t *testing.T) {
	db := setupTokenControllerTestDB(t)
	actor := &model.User{Username: "review-admin", AffCode: "review-admin-aff", Password: "password", Role: common.RoleAdminUser, Status: common.UserStatusEnabled}
	peer := &model.User{Username: "peer-admin", AffCode: "peer-admin-aff", Password: "password", Role: common.RoleAdminUser, Status: common.UserStatusEnabled}
	require.NoError(t, db.Create(actor).Error)
	require.NoError(t, db.Create(peer).Error)

	recorder := httptest.NewRecorder()
	c, _ := gin.CreateTestContext(recorder)
	c.Set("id", actor.Id)
	c.Set("role", actor.Role)
	c.Request = httptest.NewRequest(http.MethodGet, "/api/assistant/admin/request-reviews?user_id="+strconv.Itoa(peer.Id), nil)
	AdminListAssistantRequestReviews(c)
	require.Equal(t, http.StatusOK, recorder.Code)
	require.Contains(t, recorder.Body.String(), `"success":false`)
}

func TestAdminAssistantRequestReviewsExposeQueueStats(t *testing.T) {
	db := setupTokenControllerTestDB(t)
	actor := &model.User{Username: "review-admin-self", AffCode: "review-admin-self-aff", Password: "password", Role: common.RoleAdminUser, Status: common.UserStatusEnabled}
	require.NoError(t, db.Create(actor).Error)
	require.NoError(t, db.AutoMigrate(&model.AssistantRequestReview{}, &model.AssistantReviewReset{}))

	recorder := httptest.NewRecorder()
	c, _ := gin.CreateTestContext(recorder)
	c.Set("id", actor.Id)
	c.Set("role", actor.Role)
	c.Request = httptest.NewRequest(http.MethodGet, "/api/assistant/admin/request-reviews?user_id="+strconv.Itoa(actor.Id), nil)
	AdminListAssistantRequestReviews(c)
	require.Equal(t, http.StatusOK, recorder.Code)
	require.Contains(t, recorder.Body.String(), `"queue_stats"`)
}
