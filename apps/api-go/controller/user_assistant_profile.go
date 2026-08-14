package controller

import (
	"errors"
	"net/http"
	"strconv"
	"strings"

	"github.com/QuantumNous/new-api/common"
	"github.com/QuantumNous/new-api/model"
	"github.com/QuantumNous/new-api/service"
	"github.com/gin-gonic/gin"
	"gorm.io/gorm"
)

type assistantUserProfileAdminRequest struct {
	ProfileKey string   `json:"profile_key"`
	Tags       []string `json:"tags"`
	Strategy   string   `json:"strategy"`
	Enabled    bool     `json:"enabled"`
}

func adminSkillScope(c *gin.Context) (service.UserSkills, int, bool) {
	if c.GetInt("role") < common.RoleAdminUser {
		writeAssistantError(c, http.StatusForbidden, "ASSISTANT_PROFILE_FORBIDDEN", model.ErrAssistantHistoryForbidden)
		return service.UserSkills{}, 0, false
	}
	targetID, err := strconv.Atoi(strings.TrimSpace(c.Param("id")))
	if err != nil || targetID <= 0 {
		writeAssistantError(c, http.StatusBadRequest, "ASSISTANT_PROFILE_INVALID_USER", errors.New("invalid user id"))
		return service.UserSkills{}, 0, false
	}
	scope, err := service.OpenSkills(c.GetInt("id"), targetID)
	if err != nil {
		if errors.Is(err, model.ErrAssistantHistoryForbidden) {
			writeAssistantError(c, http.StatusForbidden, "ASSISTANT_PROFILE_FORBIDDEN", err)
			return service.UserSkills{}, 0, false
		}
		if errors.Is(err, model.ErrAssistantConversationNotFound) {
			writeAssistantError(c, http.StatusNotFound, "ASSISTANT_PROFILE_USER_NOT_FOUND", err)
			return service.UserSkills{}, 0, false
		}
		common.ApiError(c, err)
		return service.UserSkills{}, 0, false
	}
	return scope, targetID, true
}

// AdminGetAssistantUserProfile is intentionally mounted below the AdminAuth
// user-management router. There is no self/user-facing equivalent: profile
// tags and handling strategies are internal moderation metadata.
func AdminGetAssistantUserProfile(c *gin.Context) {
	scope, _, ok := adminSkillScope(c)
	if !ok {
		return
	}
	profile, err := scope.Profile()
	if err != nil {
		common.ApiError(c, err)
		return
	}
	common.ApiSuccess(c, model.AssistantUserProfileViewOf(profile))
}

func AdminUpdateAssistantUserProfile(c *gin.Context) {
	scope, targetID, ok := adminSkillScope(c)
	if !ok {
		return
	}
	var request assistantUserProfileAdminRequest
	if err := common.UnmarshalBodyReusable(c, &request); err != nil {
		common.ApiError(c, errors.New("invalid assistant profile payload"))
		return
	}
	profile, err := scope.SetProfile(service.ProfileDraft{
		Key: request.ProfileKey, Tags: request.Tags, Strategy: request.Strategy, Enabled: request.Enabled,
	})
	if err != nil {
		status := http.StatusBadRequest
		if errors.Is(err, gorm.ErrInvalidData) {
			status = http.StatusInternalServerError
		}
		writeAssistantError(c, status, "ASSISTANT_PROFILE_INVALID", err)
		return
	}

	// Never place tags or strategy text in the audit params. The audit records
	// that the internal policy changed without turning an admin note into a
	// second copy of potentially sensitive data.
	recordManageAuditFor(c, targetID, "assistant.user_profile_update", map[string]interface{}{
		"profile_key":    profile.ProfileKey,
		"enabled":        profile.Enabled,
		"tags_count":     len(model.AssistantUserProfileTags(profile)),
		"strategy_runes": len([]rune(profile.Strategy)),
	})
	common.ApiSuccess(c, model.AssistantUserProfileViewOf(profile))
}
