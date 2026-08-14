/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program. If not, see <https://www.gnu.org/licenses/>.
*/
package controller

import (
	"bytes"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"net/url"
	"regexp"
	"slices"
	"strings"
	"time"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/LIghtJUNction/api.lmm.best/middleware"
	"github.com/LIghtJUNction/api.lmm.best/model"
	"github.com/LIghtJUNction/api.lmm.best/relaykit/types"
	"github.com/LIghtJUNction/api.lmm.best/service"

	"github.com/gin-gonic/gin"
)

const (
	assistantDrawingPromptMaxRunes = 2000
	assistantDrawingMaxImages      = 4
)

var assistantImageRequestPattern = regexp.MustCompile(`(?i)(?:绘图|画图|生成(?:一张|图片|图像)|帮我画|generate(?: an)? image|create(?: an)? image|draw(?: an)? image)`)

type assistantDrawingDraft struct {
	Prompt  string `json:"prompt"`
	Model   string `json:"model"`
	Group   string `json:"group"`
	Size    string `json:"size,omitempty"`
	Quality string `json:"quality,omitempty"`
	N       uint   `json:"n"`
}

type assistantDrawingGenerateInput struct {
	ConfirmationToken string `json:"confirmation_token"`
}

func assistantDrawingCatalog(userGroup string) (map[string]string, map[string]struct{}) {
	groups := service.GetUserUsableGroups(userGroup)
	usableGroups := make(map[string]string, len(groups))
	for group, description := range groups {
		usableGroups[group] = description
	}

	imageModels := make(map[string]struct{})
	for _, pricing := range model.GetPricing() {
		for _, endpoint := range pricing.SupportedEndpointTypes {
			if endpoint == types.EndpointTypeImageGeneration {
				imageModels[pricing.ModelName] = struct{}{}
				break
			}
		}
	}
	return usableGroups, imageModels
}

func assistantDrawingModelsForGroup(group string, imageModels map[string]struct{}) []string {
	if group == "" {
		return nil
	}
	models := service.GetGroupsEnabledModels([]string{group})
	result := make([]string, 0, len(models))
	for _, name := range models {
		if _, ok := imageModels[name]; ok {
			result = append(result, name)
		}
	}
	slices.Sort(result)
	return result
}

func assistantDrawingModelAllowed(userGroup, group, modelID string) bool {
	groups, imageModels := assistantDrawingCatalog(userGroup)
	if _, ok := groups[group]; !ok {
		return false
	}
	return slices.Contains(assistantDrawingModelsForGroup(group, imageModels), modelID)
}

func assistantExplicitImageRequest(message string) bool {
	return assistantImageRequestPattern.MatchString(strings.TrimSpace(message))
}

func assistantImageGenerationWorkflowRequired(userContext assistantUserContext) bool {
	return common.DrawingEnabled && userContext.DeveloperAccessGranted && assistantExplicitImageRequest(userContext.LatestUserRequest)
}

func assistantImageGenerationWorkflowMinSteps(userContext assistantUserContext) int {
	if !assistantImageGenerationWorkflowRequired(userContext) {
		return 0
	}
	return 2 // prepare the confirmation card, then produce a final answer
}

func executeAssistantImageGenerationTool(c *gin.Context, userID int, input map[string]any) map[string]any {
	if !common.DrawingEnabled {
		return map[string]any{"ok": false, "status": "drawing_disabled", "error": "image generation is currently disabled"}
	}
	if c == nil || c.GetBool("use_access_token") || strings.TrimSpace(c.GetString("session_id")) == "" {
		return map[string]any{"ok": false, "status": "browser_session_required", "error": "image generation confirmation requires a browser session"}
	}
	user, err := model.GetUserCache(userID)
	if err != nil {
		return map[string]any{"ok": false, "error": "account access could not be loaded"}
	}
	access, err := model.GetDeveloperAccessStateForUserBase(user)
	if err != nil || !access.Granted {
		return map[string]any{"ok": false, "status": "l1_required", "error": "L1 access is required for image generation"}
	}
	groups, imageModels := assistantDrawingCatalog(user.Group)
	group := strings.TrimSpace(inputString(input, "group"))
	if group == "" {
		// Keep the requested image-2 preference data-driven: it is only a
		// default when the live group catalog actually exposes that group.
		if _, ok := groups["image-2"]; ok {
			group = "image-2"
		} else {
			groupNames := make([]string, 0, len(groups))
			for name := range groups {
				groupNames = append(groupNames, name)
			}
			slices.Sort(groupNames)
			return map[string]any{
				"ok": true, "status": "selection_required", "groups": groupNames,
				"next_step": "Ask the user to choose one exact routing group before preparing the image.",
			}
		}
	}
	if _, ok := groups[group]; !ok {
		return map[string]any{"ok": false, "status": "invalid_group", "error": "the requested image group is not available to this account"}
	}
	models := assistantDrawingModelsForGroup(group, imageModels)
	if len(models) == 0 {
		return map[string]any{"ok": false, "status": "no_image_models", "error": "the selected group has no image-capable models"}
	}
	modelID := strings.TrimSpace(inputString(input, "model"))
	if modelID == "" {
		if slices.Contains(models, "image-2") {
			modelID = "image-2"
		} else {
			return map[string]any{
				"ok": true, "status": "selection_required", "group": group, "model_ids": models,
				"next_step": "Ask the user to choose one exact image model from this group.",
			}
		}
	}
	if !slices.Contains(models, modelID) {
		return map[string]any{"ok": false, "status": "model_unavailable", "error": "the exact image model is not available in the selected group"}
	}
	prompt := strings.TrimSpace(inputString(input, "prompt"))
	if prompt == "" || len([]rune(prompt)) > assistantDrawingPromptMaxRunes {
		return map[string]any{"ok": false, "status": "prompt_invalid", "error": "image prompt must contain 1 to 2000 characters"}
	}
	n := uint(1)
	if value, ok := inputNumber(input, "n"); ok {
		if value < 1 || value > assistantDrawingMaxImages || value != float64(uint(value)) {
			return map[string]any{"ok": false, "status": "image_count_invalid", "error": "image count must be between 1 and 4"}
		}
		n = uint(value)
	}
	draft := assistantDrawingDraft{
		Prompt:  prompt,
		Model:   modelID,
		Group:   group,
		Size:    strings.TrimSpace(inputString(input, "size")),
		Quality: strings.TrimSpace(inputString(input, "quality")),
		N:       n,
	}
	payload, err := json.Marshal(draft)
	if err != nil {
		return map[string]any{"ok": false, "error": "image request could not be prepared"}
	}
	token, _, err := model.CreateAuthFlow(model.AuthFlowCreate{
		Purpose:   model.AuthFlowPurposeAssistantDrawing,
		UserId:    userID,
		SessionId: strings.TrimSpace(c.GetString("session_id")),
		Payload:   string(payload),
		ExpiresAt: time.Now().Add(10 * time.Minute),
	})
	if err != nil {
		return map[string]any{"ok": false, "error": "image request confirmation could not be created"}
	}
	action := map[string]any{
		"type": "image_generation", "requires_confirmation": true,
		"confirmation_token": token, "expires_in_seconds": 600,
		"prompt": prompt, "model": modelID, "group": group, "n": n,
	}
	if draft.Size != "" {
		action["size"] = draft.Size
	}
	if draft.Quality != "" {
		action["quality"] = draft.Quality
	}
	c.Set(assistantClientActionKey, action)
	return map[string]any{
		"ok": true, "status": "confirmation_required", "action": "image_generation",
		"message": "Ask the user to confirm the exact image prompt, model, routing group, and image count before generating.",
	}
}

// PlaygroundImage reuses the normal relay billing, routing and safety path,
// but binds it to the authenticated browser user's selected group.
func PlaygroundImage(c *gin.Context) {
	if !common.DrawingEnabled {
		c.JSON(http.StatusForbidden, gin.H{"error": gin.H{"message": "image generation is disabled"}})
		return
	}
	if c.GetBool("use_access_token") {
		c.JSON(http.StatusForbidden, gin.H{"error": gin.H{"message": "browser authentication is required"}})
		return
	}
	user, err := model.GetUserCache(c.GetInt("id"))
	if err != nil {
		c.JSON(http.StatusUnauthorized, gin.H{"error": gin.H{"message": "signed-in account is unavailable"}})
		return
	}
	group := strings.TrimSpace(c.Query("group"))
	if group == "" {
		group = user.Group
	}
	if _, ok := service.GetUserUsableGroups(user.Group)[group]; !ok {
		c.JSON(http.StatusForbidden, gin.H{"error": gin.H{"message": "the selected image group is not available to this account"}})
		return
	}
	if c.Request.Body == nil {
		c.JSON(http.StatusBadRequest, gin.H{"error": gin.H{"message": "an image request body is required"}})
		return
	}
	body, err := io.ReadAll(c.Request.Body)
	if err != nil {
		c.JSON(http.StatusBadRequest, gin.H{"error": gin.H{"message": "image request could not be read"}})
		return
	}
	c.Request.Body = io.NopCloser(bytes.NewReader(body))
	var request struct {
		Model  string `json:"model"`
		Prompt string `json:"prompt"`
	}
	if err := json.Unmarshal(body, &request); err != nil || strings.TrimSpace(request.Model) == "" {
		c.JSON(http.StatusBadRequest, gin.H{"error": gin.H{"message": "an exact image model is required"}})
		return
	}
	if !assistantDrawingModelAllowed(user.Group, group, strings.TrimSpace(request.Model)) {
		c.JSON(http.StatusForbidden, gin.H{"error": gin.H{"message": "the selected image model is not available in this group"}})
		return
	}
	if strings.TrimSpace(request.Prompt) == "" || len([]rune(request.Prompt)) > assistantDrawingPromptMaxRunes {
		c.JSON(http.StatusBadRequest, gin.H{"error": gin.H{"message": "image prompt must contain 1 to 2000 characters"}})
		return
	}
	user.WriteContext(c)
	tempToken := &model.Token{UserId: user.Id, Name: "drawing-workbench", Group: group}
	if err := middleware.SetupContextForToken(c, tempToken); err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": gin.H{"message": "image routing context could not be prepared"}})
		return
	}
	Relay(c, types.RelayFormatOpenAIImage)
}

// GenerateAssistantDrawing consumes the session-bound confirmation and then
// enters the same image relay used by the drawing workbench. The server never
// trusts model, group or prompt values re-sent by the browser.
func GenerateAssistantDrawing(c *gin.Context) {
	if !common.DrawingEnabled {
		c.JSON(http.StatusForbidden, gin.H{"error": gin.H{"message": "image generation is disabled"}})
		return
	}
	if c.GetBool("use_access_token") || strings.TrimSpace(c.GetString("session_id")) == "" {
		c.JSON(http.StatusForbidden, gin.H{"error": gin.H{"message": "browser authentication is required"}})
		return
	}
	var input assistantDrawingGenerateInput
	if err := common.DecodeJson(c.Request.Body, &input); err != nil || strings.TrimSpace(input.ConfirmationToken) == "" {
		c.JSON(http.StatusBadRequest, gin.H{"error": gin.H{"message": "a drawing confirmation token is required"}})
		return
	}
	flow, err := model.ConsumeAuthFlow(strings.TrimSpace(input.ConfirmationToken), model.AuthFlowMatch{
		Purpose:   model.AuthFlowPurposeAssistantDrawing,
		UserId:    c.GetInt("id"),
		SessionId: strings.TrimSpace(c.GetString("session_id")),
	})
	if err != nil {
		status := http.StatusUnprocessableEntity
		if errors.Is(err, model.ErrAuthFlowConsumed) {
			status = http.StatusConflict
		}
		c.JSON(status, gin.H{"error": gin.H{"message": "image confirmation is invalid or expired; ask the assistant to prepare it again"}})
		return
	}
	var draft assistantDrawingDraft
	if err := json.Unmarshal([]byte(flow.Payload), &draft); err != nil || draft.Prompt == "" || draft.Model == "" || draft.Group == "" {
		c.JSON(http.StatusUnprocessableEntity, gin.H{"error": gin.H{"message": "image confirmation payload is invalid"}})
		return
	}
	user, err := model.GetUserCache(c.GetInt("id"))
	if err != nil || !assistantDrawingModelAllowed(user.Group, draft.Group, draft.Model) {
		c.JSON(http.StatusUnprocessableEntity, gin.H{"error": gin.H{"message": "the confirmed image model or group is no longer available"}})
		return
	}
	body, err := json.Marshal(draft)
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": gin.H{"message": "image request could not be encoded"}})
		return
	}
	originalURL := *c.Request.URL
	query := originalURL.Query()
	query.Set("group", draft.Group)
	c.Request.URL = &url.URL{Path: "/pg/images/generations", RawQuery: query.Encode()}
	c.Request.Body = io.NopCloser(bytes.NewReader(body))
	defer func() { c.Request.URL = &originalURL }()
	PlaygroundImage(c)
}
