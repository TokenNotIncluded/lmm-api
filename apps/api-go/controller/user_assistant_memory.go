package controller

import (
	"errors"
	"net/http"
	"strconv"
	"strings"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/LIghtJUNction/api.lmm.best/model"
	"github.com/LIghtJUNction/api.lmm.best/service"
	"github.com/gin-gonic/gin"
)

type assistantMemoryAdminRequest struct {
	Title   string   `json:"title"`
	Content string   `json:"content"`
	Tags    []string `json:"tags"`
	Enabled bool     `json:"enabled"`
}

func AdminListMemories(c *gin.Context) {
	scope, _, ok := adminSkillScope(c)
	if !ok {
		return
	}
	memories, err := scope.Memories(true)
	if err != nil {
		if errors.Is(err, model.ErrAssistantHistoryForbidden) {
			writeAssistantError(c, http.StatusForbidden, "ASSISTANT_MEMORY_FORBIDDEN", err)
			return
		}
		common.ApiError(c, err)
		return
	}
	common.ApiSuccess(c, memories)
}

func saveMemoryAsAdmin(c *gin.Context, memoryID int64) {
	scope, targetID, ok := adminSkillScope(c)
	if !ok {
		return
	}
	var request assistantMemoryAdminRequest
	if err := common.UnmarshalBodyReusable(c, &request); err != nil {
		writeAssistantError(c, http.StatusBadRequest, "ASSISTANT_MEMORY_INVALID", errors.New("invalid assistant memory payload"))
		return
	}
	memory, err := scope.SetMemory(service.MemoryDraft{
		ID: memoryID, Title: request.Title, Content: request.Content,
		Tags: request.Tags, Enabled: request.Enabled,
	})
	if err != nil {
		status := http.StatusBadRequest
		if errors.Is(err, model.ErrAssistantMemoryMissing) {
			status = http.StatusNotFound
		}
		writeAssistantError(c, status, "ASSISTANT_MEMORY_INVALID", err)
		return
	}
	recordManageAuditFor(c, targetID, "assistant.user_memory_update", map[string]interface{}{
		"memory_id": memory.Id, "enabled": memory.Enabled, "tags_count": len(memory.Tags()),
		"content_runes": len([]rune(memory.Content)),
	})
	common.ApiSuccess(c, memory.View())
}

func AdminCreateMemory(c *gin.Context) {
	saveMemoryAsAdmin(c, 0)
}

func AdminUpdateMemory(c *gin.Context) {
	memoryID, err := strconv.ParseInt(strings.TrimSpace(c.Param("memoryId")), 10, 64)
	if err != nil || memoryID <= 0 {
		writeAssistantError(c, http.StatusBadRequest, "ASSISTANT_MEMORY_INVALID", errors.New("invalid assistant memory id"))
		return
	}
	saveMemoryAsAdmin(c, memoryID)
}

func AdminDeleteMemory(c *gin.Context) {
	scope, targetID, ok := adminSkillScope(c)
	if !ok {
		return
	}
	memoryID, err := strconv.ParseInt(strings.TrimSpace(c.Param("memoryId")), 10, 64)
	if err != nil || memoryID <= 0 {
		writeAssistantError(c, http.StatusBadRequest, "ASSISTANT_MEMORY_INVALID", errors.New("invalid assistant memory id"))
		return
	}
	if err := scope.Forget(memoryID); err != nil {
		status := http.StatusInternalServerError
		if errors.Is(err, model.ErrAssistantMemoryMissing) {
			status = http.StatusNotFound
		}
		writeAssistantError(c, status, "ASSISTANT_MEMORY_DELETE_FAILED", err)
		return
	}
	recordManageAuditFor(c, targetID, "assistant.user_memory_delete", map[string]interface{}{"memory_id": memoryID})
	common.ApiSuccess(c, gin.H{"deleted": true, "memory_id": memoryID})
}
