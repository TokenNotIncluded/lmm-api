package controller

import (
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"sort"
	"strconv"
	"strings"
	"time"
	"unicode/utf8"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/LIghtJUNction/api.lmm.best/model"
	"github.com/LIghtJUNction/api.lmm.best/service"
	"github.com/LIghtJUNction/api.lmm.best/setting"
	"github.com/LIghtJUNction/api.lmm.best/setting/operation_setting"
	"github.com/gin-gonic/gin"
)

type assistantCreateKeyInput struct {
	Confirmed         bool   `json:"confirmed"`
	ConfirmationToken string `json:"confirmation_token"`
	Name              string `json:"name"`
	Group             string `json:"group"`
	ConversationID    int64  `json:"conversation_id"`
}

type assistantCreateKeyDraft struct {
	Name           string `json:"name"`
	Group          string `json:"group"`
	ConversationID int64  `json:"conversation_id"`
}

const assistantKeyConfirmationTTL = 10 * time.Minute

type assistantKeyGroupOption struct {
	ID            string   `json:"id"`
	Description   string   `json:"description"`
	Automatic     bool     `json:"automatic"`
	RoutingGroups []string `json:"routing_groups,omitempty"`
}

type assistantHandoffInput struct {
	Confirmed bool   `json:"confirmed"`
	Message   string `json:"message"`
}

type assistantResolveHandoffInput struct {
	Note string `json:"note"`
}

func requireAssistantBrowserSession(c *gin.Context) bool {
	if c.GetBool("use_access_token") {
		writeAssistantError(c, http.StatusForbidden, "ASSISTANT_SESSION_REQUIRED", errors.New("assistant tools require a browser login session"))
		return false
	}
	return true
}

func requireAssistantConfirmation(c *gin.Context, confirmed bool) bool {
	if !confirmed {
		writeAssistantError(c, http.StatusUnprocessableEntity, "ASSISTANT_CONFIRMATION_REQUIRED", errors.New("explicit confirmation is required"))
		return false
	}
	return true
}

func GetAssistantPricing(c *gin.Context) {
	if !requireAssistantBrowserSession(c) {
		return
	}
	if result, blocked := assistantDeveloperCapabilityRequired(c.GetInt("id"), "live model pricing"); blocked {
		message := inputString(result, "error")
		if message == "" {
			message = "L1 access is required for live model pricing"
		}
		writeAssistantError(c, http.StatusForbidden, "ASSISTANT_L1_REQUIRED", errors.New(message))
		return
	}
	status, response := buildPricingResponse(c, true)
	c.JSON(status, response)
}

func GetAssistantPlanOffers(c *gin.Context) {
	if !requireAssistantBrowserSession(c) {
		return
	}
	common.ApiSuccess(c, executeAssistantPlanOffersTool(c.GetInt("id")))
}

func getAssistantDeveloperAccess(userID int) (*model.UserBase, bool, error) {
	user, err := model.GetUserCache(userID)
	if err != nil {
		return nil, false, err
	}
	access, err := model.GetDeveloperAccessStateForUserBase(user)
	if err != nil {
		return nil, false, err
	}
	return user, access.Granted, nil
}

func getAssistantKeyGroupOptions(userGroup string) []assistantKeyGroupOption {
	usableGroups := service.GetUserUsableGroups(userGroup)
	groupIDs := make([]string, 0, len(usableGroups))
	for groupID := range usableGroups {
		if service.IsUserSelectableGroup(userGroup, groupID) {
			groupIDs = append(groupIDs, groupID)
		}
	}
	sort.Strings(groupIDs)

	options := make([]assistantKeyGroupOption, 0, len(groupIDs)+1)
	if routingGroups := service.GetUserAutoGroup(userGroup); len(routingGroups) > 0 {
		options = append(options, assistantKeyGroupOption{
			ID:            "auto",
			Description:   "Automatic routing across the listed groups",
			Automatic:     true,
			RoutingGroups: routingGroups,
		})
	}
	for _, groupID := range groupIDs {
		options = append(options, assistantKeyGroupOption{
			ID:          groupID,
			Description: usableGroups[groupID],
		})
	}
	return options
}

func executeAssistantCreateKeyRequestTool(c *gin.Context, userID int, input map[string]any) map[string]any {
	if c == nil || c.GetBool("use_access_token") {
		return map[string]any{"ok": false, "error": "a browser login session is required to prepare API key creation"}
	}
	user, granted, err := getAssistantDeveloperAccess(userID)
	if err != nil {
		return map[string]any{"ok": false, "error": "account access could not be loaded"}
	}
	if !granted {
		return map[string]any{"ok": false, "error": "L1 access is required to create an API key"}
	}

	name := strings.TrimSpace(inputString(input, "name"))
	if name == "" {
		name = "AI assistant key"
	}
	if utf8.RuneCountInString(name) > 50 {
		return map[string]any{"ok": false, "status": "name_invalid", "error": "API key name must be at most 50 characters"}
	}
	options := getAssistantKeyGroupOptions(user.Group)
	group := inputString(input, "group")
	if assistantUserContextFromGin(c).CreateKeyAction == assistantCreateKeyActionRequest {
		// The first model turn may discover choices but may not choose one on the
		// user's behalf. A later user turn naming the group is required.
		group = ""
	}
	if group == "" {
		return map[string]any{
			"ok":               true,
			"status":           "group_required",
			"action":           "create_key",
			"available_groups": options,
			"message":          "Ask the user to choose one exact routing group before requesting confirmation.",
			"requested_name":   name,
		}
	}
	if (group != "auto" && !service.IsUserSelectableGroup(user.Group, group)) ||
		(group == "auto" && len(service.GetUserAutoGroup(user.Group)) == 0) {
		return map[string]any{
			"ok":               false,
			"status":           "invalid_group",
			"error":            "the selected group is not available for this account",
			"available_groups": options,
		}
	}
	sessionID := strings.TrimSpace(c.GetString("session_id"))
	if sessionID == "" {
		return map[string]any{"ok": false, "status": "session_required", "error": "a browser login session is required to prepare API key creation"}
	}
	conversationID := assistantHistoryConversationID(c)
	payload, err := json.Marshal(assistantCreateKeyDraft{Name: name, Group: group, ConversationID: conversationID})
	if err != nil {
		return map[string]any{"ok": false, "error": "API key confirmation could not be prepared"}
	}
	confirmationToken, _, err := model.CreateAuthFlow(model.AuthFlowCreate{
		Purpose: model.AuthFlowPurposeAssistantKey, UserId: userID, SessionId: sessionID,
		Payload: string(payload), ExpiresAt: time.Now().Add(assistantKeyConfirmationTTL),
	})
	if err != nil {
		return map[string]any{"ok": false, "error": "API key confirmation could not be prepared"}
	}
	action := map[string]any{
		"type": "create_key", "confirmation_token": confirmationToken,
		"requires_confirmation": true, "expires_in_seconds": int(assistantKeyConfirmationTTL / time.Second),
		"name": name, "group": group, "conversation_id": conversationID, "ui_path": "/keys",
	}
	c.Set(assistantClientActionKey, action)
	return map[string]any{
		"ok":              true,
		"status":          "confirmation_required",
		"action":          "create_key",
		"ui_path":         "/keys",
		"message":         "Ask the user to explicitly confirm creating the key with this exact group; do not claim that a key exists yet.",
		"requested_name":  name,
		"requested_group": group,
	}
}

// CreateAssistantDefaultKey creates the same safe unlimited, non-expiring key
// offered by the regular key form. It is deliberately confirmation-gated and
// restricted to L1+ browser sessions.
func CreateAssistantDefaultKey(c *gin.Context) {
	if !requireAssistantBrowserSession(c) {
		return
	}
	var input assistantCreateKeyInput
	if err := c.ShouldBindJSON(&input); err != nil {
		writeAssistantError(c, http.StatusBadRequest, "ASSISTANT_INVALID_REQUEST", errors.New("invalid key creation request"))
		return
	}
	userID := c.GetInt("id")
	user, granted, err := getAssistantDeveloperAccess(userID)
	if err != nil {
		common.ApiError(c, err)
		return
	}
	if !granted {
		writeAssistantError(c, http.StatusForbidden, "ASSISTANT_L1_REQUIRED", errors.New("L1 access is required to create an API key"))
		return
	}

	confirmationToken := strings.TrimSpace(input.ConfirmationToken)
	name := strings.TrimSpace(input.Name)
	group := strings.TrimSpace(input.Group)
	flowMatch := model.AuthFlowMatch{}
	if confirmationToken != "" {
		sessionID := strings.TrimSpace(c.GetString("session_id"))
		if sessionID == "" {
			writeAssistantError(c, http.StatusForbidden, "ASSISTANT_SESSION_REQUIRED", errors.New("a browser login session is required for assistant key confirmation"))
			return
		}
		flowMatch = model.AuthFlowMatch{
			Purpose: model.AuthFlowPurposeAssistantKey, UserId: userID, SessionId: sessionID,
		}
		flow, flowErr := model.GetAuthFlow(confirmationToken, flowMatch)
		if flowErr != nil {
			writeAssistantError(c, http.StatusUnprocessableEntity, "ASSISTANT_KEY_CONFIRMATION_INVALID", errors.New("API key confirmation is invalid, expired, or already used"))
			return
		}
		var draft assistantCreateKeyDraft
		if json.Unmarshal([]byte(flow.Payload), &draft) != nil {
			writeAssistantError(c, http.StatusUnprocessableEntity, "ASSISTANT_KEY_CONFIRMATION_INVALID", errors.New("API key confirmation is invalid"))
			return
		}
		// The browser confirms only the opaque draft. Never trust client-side
		// replacements for the assistant-prepared name or group.
		name = strings.TrimSpace(draft.Name)
		group = strings.TrimSpace(draft.Group)
		input.ConversationID = draft.ConversationID
	}
	if name == "" {
		name = "AI assistant key"
	}
	if utf8.RuneCountInString(name) > 50 {
		writeAssistantError(c, http.StatusUnprocessableEntity, "ASSISTANT_KEY_NAME_TOO_LONG", errors.New("API key name must be at most 50 characters"))
		return
	}
	if utf8.RuneCountInString(group) > 64 {
		writeAssistantError(c, http.StatusUnprocessableEntity, "ASSISTANT_KEY_GROUP_TOO_LONG", errors.New("API key group must be at most 64 characters"))
		return
	}
	if group == "" {
		c.AbortWithStatusJSON(http.StatusUnprocessableEntity, gin.H{
			"success":          false,
			"code":             "ASSISTANT_KEY_GROUP_REQUIRED",
			"message":          "choose a routing group before confirming key creation",
			"available_groups": getAssistantKeyGroupOptions(user.Group),
		})
		return
	}
	if group != "auto" && !service.IsUserSelectableGroup(user.Group, group) {
		writeAssistantError(c, http.StatusUnprocessableEntity, "ASSISTANT_INVALID_GROUP", errors.New("the selected group is not available for this account"))
		return
	}
	if group == "auto" && len(service.GetUserAutoGroup(user.Group)) == 0 {
		writeAssistantError(c, http.StatusUnprocessableEntity, "ASSISTANT_INVALID_GROUP", errors.New("automatic routing is not available for this account"))
		return
	}
	if !requireAssistantConfirmation(c, input.Confirmed) {
		return
	}
	maxTokens := operation_setting.GetMaxUserTokens()
	if confirmationToken == "" {
		count, countErr := model.CountUserTokens(userID)
		if countErr != nil {
			common.ApiError(c, countErr)
			return
		}
		if int(count) >= maxTokens {
			writeAssistantError(c, http.StatusConflict, "ASSISTANT_KEY_LIMIT_REACHED", fmt.Errorf("API key limit reached (%d)", maxTokens))
			return
		}
	}

	key, err := common.GenerateKey()
	if err != nil {
		writeAssistantError(c, http.StatusInternalServerError, "ASSISTANT_KEY_GENERATE_FAILED", errors.New("failed to generate API key"))
		return
	}
	now := common.GetTimestamp()
	token := model.Token{
		UserId:             userID,
		Name:               name,
		Key:                key,
		CreatedTime:        now,
		AccessedTime:       now,
		ExpiredTime:        -1,
		UnlimitedQuota:     true,
		ModelLimitsEnabled: false,
	}
	if group != "" {
		token.Group = group
		token.CrossGroupRetry = group == "auto"
	} else if setting.DefaultUseAutoGroup && len(service.GetUserAutoGroup(user.Group)) > 0 {
		token.Group = "auto"
		token.CrossGroupRetry = true
	}
	var card *model.AssistantSecureCard
	if confirmationToken != "" {
		card, err = model.ConsumeAssistantKeyFlowAndCreateSecureCard(
			confirmationToken, flowMatch, &token, userID, input.ConversationID,
			"已创建 API 凭证；仅你可一次性查看和复制",
			fmt.Sprintf(`{"api_key":%q}`, "sk-"+token.Key), maxTokens,
		)
	} else {
		card, err = model.InsertAssistantTokenAndCreateSecureCard(
			&token, userID, input.ConversationID,
			"已创建 API 凭证；仅你可一次性查看和复制",
			fmt.Sprintf(`{"api_key":%q}`, "sk-"+token.Key),
		)
	}
	if err != nil {
		if errors.Is(err, model.ErrAuthFlowInvalid) || errors.Is(err, model.ErrAuthFlowExpired) || errors.Is(err, model.ErrAuthFlowConsumed) {
			writeAssistantError(c, http.StatusUnprocessableEntity, "ASSISTANT_KEY_CONFIRMATION_INVALID", errors.New("API key confirmation is invalid, expired, or already used"))
			return
		}
		if errors.Is(err, model.ErrAssistantTokenLimit) {
			writeAssistantError(c, http.StatusConflict, "ASSISTANT_KEY_LIMIT_REACHED", fmt.Errorf("API key limit reached (%d)", maxTokens))
			return
		}
		// Never fall back to serializing the key in a normal JSON response. The
		// model transaction rolls back the token as well as the card on failure.
		writeAssistantError(c, http.StatusInternalServerError, "ASSISTANT_SECURE_CARD_CREATE_FAILED", errors.New("API key could not be created securely; please try again"))
		return
	}
	model.RecordLog(userID, model.LogTypeSystem, fmt.Sprintf("created API key %d via assistant", token.Id))
	common.ApiSuccess(c, gin.H{
		"id":             token.Id,
		"name":           token.Name,
		"group":          token.Group,
		"expired_time":   token.ExpiredTime,
		"card":           model.AssistantSecureCardViewForOwner(card),
		"privacy_notice": model.AssistantHistoryPrivacyNotice,
	})
}

func SubmitAssistantHandoff(c *gin.Context) {
	if !requireAssistantBrowserSession(c) {
		return
	}
	var input assistantHandoffInput
	if err := c.ShouldBindJSON(&input); err != nil {
		writeAssistantError(c, http.StatusBadRequest, "ASSISTANT_INVALID_REQUEST", errors.New("invalid support request"))
		return
	}
	if !requireAssistantConfirmation(c, input.Confirmed) {
		return
	}
	lead, err := model.SubmitAssistantHandoff(c.GetInt("id"), input.Message)
	if err != nil {
		switch {
		case errors.Is(err, model.ErrAssistantHandoffMessageRequired),
			errors.Is(err, model.ErrAssistantHandoffMessageTooShort),
			errors.Is(err, model.ErrAssistantHandoffMessageTooLong):
			writeAssistantError(c, http.StatusUnprocessableEntity, "ASSISTANT_HANDOFF_INVALID_MESSAGE", err)
		default:
			common.ApiError(c, err)
		}
		return
	}
	model.RecordLog(c.GetInt("id"), model.LogTypeSystem, fmt.Sprintf("submitted assistant support request %d", lead.Id))
	common.ApiSuccess(c, lead)
}

func GetAssistantHandoff(c *gin.Context) {
	lead, err := model.GetLatestAssistantHandoff(c.GetInt("id"))
	if err != nil {
		common.ApiError(c, err)
		return
	}
	common.ApiSuccess(c, lead)
}

func AdminListAssistantHandoffs(c *gin.Context) {
	leads, err := model.ListAssistantHandoffs(c.Query("status"), 100)
	if err != nil {
		if errors.Is(err, model.ErrAssistantLeadStatus) {
			writeAssistantError(c, http.StatusBadRequest, "ASSISTANT_HANDOFF_INVALID_STATUS", err)
			return
		}
		common.ApiError(c, err)
		return
	}
	common.ApiSuccess(c, leads)
}

func assistantSummarySince(c *gin.Context, errorCode string) (int64, bool) {
	days := 30
	if raw := strings.TrimSpace(c.Query("days")); raw != "" {
		parsed, err := strconv.Atoi(raw)
		if err != nil || parsed < 1 || parsed > 365 {
			writeAssistantError(c, http.StatusBadRequest, errorCode, errors.New("days must be between 1 and 365"))
			return 0, false
		}
		days = parsed
	}
	return time.Now().Add(-time.Duration(days) * 24 * time.Hour).Unix(), true
}

func AdminGetAssistantIntentSummary(c *gin.Context) {
	since, ok := assistantSummarySince(c, "ASSISTANT_INTENT_DAYS_INVALID")
	if !ok {
		return
	}
	summary, err := model.ListAssistantIntentSummary(since)
	if err != nil {
		common.ApiError(c, err)
		return
	}
	common.ApiSuccess(c, summary)
}

func AdminGetAssistantFirstQuestionSummary(c *gin.Context) {
	since, ok := assistantSummarySince(c, "ASSISTANT_FIRST_QUESTION_DAYS_INVALID")
	if !ok {
		return
	}
	summary, err := model.ListAssistantFirstQuestionSummary(since)
	if err != nil {
		common.ApiError(c, err)
		return
	}
	common.ApiSuccess(c, summary)
}

func AdminGetAssistantProfileSummary(c *gin.Context) {
	since, ok := assistantSummarySince(c, "ASSISTANT_PROFILE_DAYS_INVALID")
	if !ok {
		return
	}
	summary, err := model.ListAssistantProfileSummary(since)
	if err != nil {
		common.ApiError(c, err)
		return
	}
	common.ApiSuccess(c, summary)
}

func AdminGetAssistantReview(c *gin.Context) {
	task, err := model.GetLatestSystemTask(model.SystemTaskTypeAssistantReview)
	if err != nil {
		common.ApiError(c, err)
		return
	}
	if task == nil {
		common.ApiSuccess(c, nil)
		return
	}
	common.ApiSuccess(c, task.ToResponse())
}

func AdminRunAssistantReview(c *gin.Context) {
	task, err := service.StartAssistantReview()
	if err != nil {
		common.ApiError(c, err)
		return
	}
	common.ApiSuccess(c, task.ToResponse())
}

func AdminGetAssistantFundingSummary(c *gin.Context) {
	since, ok := assistantSummarySince(c, "ASSISTANT_FUNDING_DAYS_INVALID")
	if !ok {
		return
	}
	billingUser, err := loadAssistantBillingUser()
	if err != nil || billingUser == nil {
		writeAssistantError(c, http.StatusServiceUnavailable, "ASSISTANT_BILLING_ACCOUNT_UNAVAILABLE", errors.New("AI assistant billing account is unavailable"))
		return
	}
	end := time.Now().Unix()
	summary, err := model.GetAssistantFundingSummary(billingUser.Id, since, end)
	if err != nil {
		common.ApiError(c, err)
		return
	}
	remainingQuota, err := model.GetUserQuota(billingUser.Id, true)
	if err != nil {
		common.ApiError(c, err)
		return
	}
	remainingUSD := float64(remainingQuota)
	if common.QuotaPerUnit > 0 {
		remainingUSD /= common.QuotaPerUnit
	}
	common.ApiSuccess(c, gin.H{
		"start_timestamp":   summary.StartTimestamp,
		"end_timestamp":     summary.EndTimestamp,
		"requests":          summary.Requests,
		"prompt_tokens":     summary.PromptTokens,
		"completion_tokens": summary.CompletionTokens,
		"total_tokens":      summary.TotalTokens,
		"quota":             summary.Quota,
		"cost_usd":          summary.CostUSD,
		"remaining_quota":   remainingQuota,
		"remaining_usd":     remainingUSD,
	})
}

func AdminResolveAssistantHandoff(c *gin.Context) {
	leadID, err := strconv.Atoi(c.Param("id"))
	if err != nil || leadID <= 0 {
		writeAssistantError(c, http.StatusBadRequest, "ASSISTANT_HANDOFF_INVALID_ID", errors.New("invalid support request id"))
		return
	}
	var input assistantResolveHandoffInput
	if c.Request.ContentLength != 0 {
		if err := c.ShouldBindJSON(&input); err != nil {
			writeAssistantError(c, http.StatusBadRequest, "ASSISTANT_INVALID_REQUEST", errors.New("invalid support resolution"))
			return
		}
	}
	lead, err := model.ResolveAssistantHandoff(c.GetInt("id"), leadID, input.Note)
	if err != nil {
		switch {
		case errors.Is(err, model.ErrAssistantLeadNotFound):
			writeAssistantError(c, http.StatusNotFound, "ASSISTANT_HANDOFF_NOT_FOUND", err)
		case errors.Is(err, model.ErrAssistantLeadAlreadyResolved):
			writeAssistantError(c, http.StatusConflict, "ASSISTANT_HANDOFF_ALREADY_RESOLVED", err)
		case errors.Is(err, model.ErrAssistantAdminNoteTooLong):
			writeAssistantError(c, http.StatusUnprocessableEntity, "ASSISTANT_HANDOFF_NOTE_TOO_LONG", err)
		default:
			common.ApiError(c, err)
		}
		return
	}
	model.RecordLog(c.GetInt("id"), model.LogTypeSystem, fmt.Sprintf("resolved assistant support request %d", lead.Id))
	common.ApiSuccess(c, lead)
}
