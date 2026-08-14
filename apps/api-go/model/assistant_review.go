package model

import (
	"context"
	"errors"
	"sort"
)

const reviewListLimit = 20

type AssistantPresetReview struct {
	PresetID        string `json:"preset_id"`
	Clicks          int64  `json:"clicks"`
	Conversations   int64  `json:"conversations"`
	Recommendations int64  `json:"recommendations"`
	Approvals       int64  `json:"approvals"`
}

type AssistantReviewAction struct {
	Code  string `json:"code"`
	Count int64  `json:"count"`
}

// AssistantReview is aggregate-only. It deliberately excludes user IDs,
// questions, transcripts, email addresses, profile strategies, and memory.
type AssistantReview struct {
	WindowStart      int64                     `json:"window_start"`
	WindowEnd        int64                     `json:"window_end"`
	ObservedAt       int64                     `json:"observed_at"`
	Intents          []AssistantIntentSummary  `json:"intents"`
	Profiles         []AssistantProfileSummary `json:"profiles"`
	Presets          []AssistantPresetReview   `json:"presets"`
	CurrentSupport   int64                     `json:"current_pending_support"`
	CurrentIncidents int64                     `json:"current_open_security_incidents"`
	Actions          []AssistantReviewAction   `json:"actions"`
}

func trimReview[T any](rows []T) []T {
	if len(rows) > reviewListLimit {
		return rows[:reviewListLimit]
	}
	return rows
}

func reviewPresets(ctx context.Context, since, until int64) ([]AssistantPresetReview, error) {
	rows := make([]AssistantPresetReview, 0, maxPromptPresets)
	err := DB.WithContext(ctx).Model(&PromptPresetStat{}).
		Select("preset_id, SUM(click_count) AS clicks, SUM(conversation_count) AS conversations, SUM(recommendation_count) AS recommendations, SUM(approval_count) AS approvals").
		Where("bucket_start >= ? AND bucket_start <= ?", since, until).
		Group("preset_id").
		Order("approvals DESC, recommendations DESC, conversations DESC, clicks DESC, preset_id ASC").
		Limit(reviewListLimit).
		Scan(&rows).Error
	return rows, err
}

func reviewCount(ctx context.Context, table any, query string, args ...any) (int64, error) {
	var count int64
	err := DB.WithContext(ctx).Model(table).Where(query, args...).Count(&count).Error
	return count, err
}

func reviewActions(review AssistantReview) []AssistantReviewAction {
	actions := make([]AssistantReviewAction, 0, 4)
	if review.CurrentSupport > 0 {
		actions = append(actions, AssistantReviewAction{Code: "review_support_queue", Count: review.CurrentSupport})
	}
	if review.CurrentIncidents > 0 {
		actions = append(actions, AssistantReviewAction{Code: "review_security_incidents", Count: review.CurrentIncidents})
	}

	var profiles, unknown int64
	for _, row := range review.Profiles {
		profiles += row.Count
		if row.Profile == AssistantProfileUnknown {
			unknown = row.Count
		}
	}
	if profiles > 0 && unknown*100 >= profiles*40 {
		actions = append(actions, AssistantReviewAction{Code: "improve_profile_classification", Count: unknown})
	}

	var clicks, conversations, recommendations, approvals int64
	for _, row := range review.Presets {
		clicks += row.Clicks
		conversations += row.Conversations
		recommendations += row.Recommendations
		approvals += row.Approvals
	}
	if clicks >= 5 && conversations*100 < clicks*40 {
		actions = append(actions, AssistantReviewAction{Code: "improve_pre_conversation_prompts", Count: clicks - conversations})
	}
	if recommendations >= 3 && approvals*100 < recommendations*30 {
		actions = append(actions, AssistantReviewAction{Code: "review_recommendation_quality", Count: recommendations - approvals})
	}

	sort.SliceStable(actions, func(i, j int) bool {
		if actions[i].Count != actions[j].Count {
			return actions[i].Count > actions[j].Count
		}
		return actions[i].Code < actions[j].Code
	})
	return actions
}

func BuildAssistantReview(ctx context.Context, start, end int64) (AssistantReview, error) {
	if DB == nil || start <= 0 || end < start {
		return AssistantReview{}, errors.New("invalid assistant review window")
	}
	if ctx == nil {
		ctx = context.Background()
	}
	review := AssistantReview{WindowStart: start, WindowEnd: end, ObservedAt: end}
	var err error
	if review.Intents, err = listAssistantIntents(ctx, start, end); err != nil {
		return AssistantReview{}, err
	}
	review.Intents = trimReview(review.Intents)
	if review.Profiles, err = listAssistantProfiles(ctx, start, end); err != nil {
		return AssistantReview{}, err
	}
	review.Profiles = trimReview(review.Profiles)
	if review.Presets, err = reviewPresets(ctx, start, end); err != nil {
		return AssistantReview{}, err
	}
	if review.CurrentSupport, err = reviewCount(ctx, &AssistantLead{}, "source = ? AND status = ?", AssistantLeadSourceHandoff, AssistantLeadStatusPending); err != nil {
		return AssistantReview{}, err
	}
	if review.CurrentIncidents, err = reviewCount(ctx, &AssistantSecurityIncident{}, "status = ?", AssistantSecurityIncidentStatusOpen); err != nil {
		return AssistantReview{}, err
	}
	review.Actions = reviewActions(review)
	return review, nil
}
