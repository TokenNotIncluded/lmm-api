package controller

import (
	"errors"
	"net/http"
	"strconv"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/LIghtJUNction/api.lmm.best/middleware"
	"github.com/LIghtJUNction/api.lmm.best/model"
	"github.com/gin-gonic/gin"
	"gorm.io/gorm"
)

type publishReleaseNoteInput struct {
	Version string `json:"version"`
	Content string `json:"content"`
}

func releaseNoteError(c *gin.Context, status int, code string, message string) {
	c.AbortWithStatusJSON(status, gin.H{
		"success": false,
		"code":    code,
		"message": message,
	})
}

func GetLatestUnreadReleaseNote(c *gin.Context) {
	var sessionCreatedAt int64
	if identity, ok := middleware.GetSessionAuthIdentity(c); ok {
		session, err := model.GetUserSessionCached(identity.SessionID)
		if err != nil {
			common.ApiError(c, err)
			return
		}
		sessionCreatedAt = session.CreatedAt
	}
	note, err := model.GetLatestUnreadReleaseNote(c.GetInt("id"), sessionCreatedAt)
	if err != nil {
		common.ApiError(c, err)
		return
	}
	common.ApiSuccess(c, note)
}

func MarkReleaseNoteRead(c *gin.Context) {
	noteID, err := strconv.Atoi(c.Param("id"))
	if err != nil || noteID <= 0 {
		releaseNoteError(c, http.StatusBadRequest, "RELEASE_NOTE_INVALID_ID", "invalid release note id")
		return
	}
	if err := model.MarkReleaseNoteRead(c.GetInt("id"), noteID); err != nil {
		if errors.Is(err, model.ErrReleaseNoteNotFound) {
			releaseNoteError(c, http.StatusNotFound, "RELEASE_NOTE_NOT_FOUND", err.Error())
			return
		}
		common.ApiError(c, err)
		return
	}
	common.ApiSuccess(c, nil)
}

func ListAdminReleaseNotes(c *gin.Context) {
	limit, _ := strconv.Atoi(c.DefaultQuery("limit", "50"))
	notes, err := model.ListReleaseNotes(limit)
	if err != nil {
		common.ApiError(c, err)
		return
	}
	common.ApiSuccess(c, notes)
}

func PublishAdminReleaseNote(c *gin.Context) {
	var input publishReleaseNoteInput
	if err := c.ShouldBindJSON(&input); err != nil {
		releaseNoteError(c, http.StatusUnprocessableEntity, "RELEASE_NOTE_INVALID_REQUEST", "invalid release note request")
		return
	}
	note, err := model.PublishReleaseNote(c.GetInt("id"), input.Version, input.Content)
	if err != nil {
		switch {
		case errors.Is(err, model.ErrReleaseNoteVersionRequired),
			errors.Is(err, model.ErrReleaseNoteVersionTooLong),
			errors.Is(err, model.ErrReleaseNoteVersionInvalid),
			errors.Is(err, model.ErrReleaseNoteContentRequired),
			errors.Is(err, model.ErrReleaseNoteContentTooLong):
			releaseNoteError(c, http.StatusUnprocessableEntity, "RELEASE_NOTE_VALIDATION_FAILED", err.Error())
			return
		case errors.Is(err, gorm.ErrInvalidData):
			releaseNoteError(c, http.StatusBadRequest, "RELEASE_NOTE_INVALID_PUBLISHER", "invalid release publisher")
			return
		default:
			common.ApiError(c, err)
			return
		}
	}
	common.ApiSuccess(c, note)
}
