package controller

import (
	"errors"
	"net/http"
	"strings"
	"time"

	"github.com/QuantumNous/new-api/common"
	"github.com/QuantumNous/new-api/model"
	"github.com/gin-gonic/gin"
)

func writeL1OnboardingError(c *gin.Context, status int, code string, err error) {
	c.AbortWithStatusJSON(status, gin.H{
		"success": false,
		"code":    code,
		"message": err.Error(),
	})
}

func l1OnboardingError(c *gin.Context, err error) {
	switch {
	case errors.Is(err, model.ErrL1OnboardingNotEligible):
		writeL1OnboardingError(c, http.StatusForbidden, "L1_ONBOARDING_NOT_ELIGIBLE", err)
	case errors.Is(err, model.ErrL1OnboardingInvalidStep):
		writeL1OnboardingError(c, http.StatusUnprocessableEntity, "L1_ONBOARDING_INVALID_STEP", err)
	case errors.Is(err, model.ErrL1OnboardingOutOfOrder):
		writeL1OnboardingError(c, http.StatusConflict, "L1_ONBOARDING_OUT_OF_ORDER", err)
	case errors.Is(err, model.ErrL1OnboardingProofRequired):
		writeL1OnboardingError(c, http.StatusForbidden, "L1_ONBOARDING_PROOF_REQUIRED", err)
	case errors.Is(err, model.ErrL1OnboardingInvalidProof):
		writeL1OnboardingError(c, http.StatusUnprocessableEntity, "L1_ONBOARDING_INVALID_PROOF", err)
	default:
		common.ApiError(c, err)
	}
}

// GetL1OnboardingTodo returns the checklist only after the current server-side
// access decision says the user has L1/developer access. L0 receives an empty,
// unavailable state and no database row is created.
func GetL1OnboardingTodo(c *gin.Context) {
	view, err := model.GetL1OnboardingTodo(c.GetInt("id"))
	if err != nil {
		l1OnboardingError(c, err)
		return
	}
	common.ApiSuccess(c, view)
}

// PatchL1OnboardingTodo is intentionally a refresh endpoint. It accepts no
// completion flag and therefore cannot be used by a browser to forge a step.
// Actual client milestones are recorded by the API-key-authenticated proof
// route below; key creation and the first successful response are derived from
// persisted server facts.
func PatchL1OnboardingTodo(c *gin.Context) {
	view, err := model.RefreshL1OnboardingTodo(c.GetInt("id"))
	if err != nil {
		l1OnboardingError(c, err)
		return
	}
	common.ApiSuccess(c, view)
}

// PostL1OnboardingProof is called by a configured client using the user's API
// key. TokenAuth has already verified the key, ownership, enabled state, and
// L1/developer access before this handler runs.
func PostL1OnboardingProof(c *gin.Context) {
	var proof model.L1OnboardingProof
	if err := c.ShouldBindJSON(&proof); err != nil {
		l1OnboardingError(c, model.ErrL1OnboardingInvalidProof)
		return
	}
	view, err := model.ApplyL1OnboardingProof(c.GetInt("id"), c.GetInt("token_id"), proof, time.Now().Unix())
	if err != nil {
		l1OnboardingError(c, err)
		return
	}
	recordUserSecurityAudit(c, c.GetInt("id"), "onboarding.todo_progress", map[string]interface{}{
		"step":     strings.TrimSpace(proof.Step),
		"source":   "api_key_proof",
		"client":   strings.TrimSpace(proof.Client),
		"token_id": c.GetInt("token_id"),
	})
	common.ApiSuccess(c, view)
}
