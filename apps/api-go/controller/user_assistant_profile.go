package controller

import (
	"errors"
	"net/http"
	"strconv"
	"strings"

	"github.com/QuantumNous/new-api/common"
	"github.com/QuantumNous/new-api/model"
	"github.com/gin-gonic/gin"
	"gorm.io/gorm"
)

type assistantUserProfileAdminRequest struct {
	ProfileKey string   `json:"profile_key"`
	Tags       []string `json:"tags"`
	Strategy   string   `json:"strategy"`
	Enabled    bool     `json:"enabled"`
}

func assistantUserProfileTarget(c *gin.Context) (*model.User, bool) {
	targetID, err := strconv.Atoi(strings.TrimSpace(c.Param("id")))
	if err != nil || targetID <= 0 {
		writeAssistantError(c, http.StatusBadRequest, "ASSISTANT_PROFILE_INVALID_USER", errors.New("invalid user id"))
		return nil, false
	}
	target, err := model.GetUserById(targetID, false)
	if err != nil {
		if errors.Is(err, gorm.ErrRecordNotFound) {
			common.ApiError(c, err)
			return nil, false
		}
		common.ApiError(c, err)
		return nil, false
	}
	if !canManageTargetRole(c.GetInt("role"), target.Role) {
		writeAssistantError(c, http.StatusForbidden, "ASSISTANT_PROFILE_FORBIDDEN", errors.New("administrator cannot manage a user at the same or higher role"))
		return nil, false
	}
	return target, true
}

// AdminGetAssistantUserProfile is intentionally mounted below the AdminAuth
// user-management router. There is no self/user-facing equivalent: profile
// tags and handling strategies are internal moderation metadata.
func AdminGetAssistantUserProfile(c *gin.Context) {
	if _, ok := assistantUserProfileTarget(c); !ok {
		return
	}
	targetID, _ := strconv.Atoi(c.Param("id"))
	profile, err := model.GetAssistantUserProfile(targetID)
	if err != nil {
		common.ApiError(c, err)
		return
	}
	common.ApiSuccess(c, model.AssistantUserProfileViewOf(profile))
}

func AdminUpdateAssistantUserProfile(c *gin.Context) {
	target, ok := assistantUserProfileTarget(c)
	if !ok {
		return
	}
	var request assistantUserProfileAdminRequest
	if err := common.UnmarshalBodyReusable(c, &request); err != nil {
		common.ApiError(c, errors.New("invalid assistant profile payload"))
		return
	}
	profile, err := model.UpsertAssistantUserProfile(
		target.Id,
		c.GetInt("id"),
		request.ProfileKey,
		request.Tags,
		request.Strategy,
		request.Enabled,
	)
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
	recordManageAuditFor(c, target.Id, "assistant.user_profile_update", map[string]interface{}{
		"profile_key":    profile.ProfileKey,
		"enabled":        profile.Enabled,
		"tags_count":     len(model.AssistantUserProfileTags(profile)),
		"strategy_runes": len([]rune(profile.Strategy)),
	})
	common.ApiSuccess(c, model.AssistantUserProfileViewOf(profile))
}
