package controller

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strconv"
	"testing"

	"github.com/QuantumNous/new-api/model"
	"github.com/gin-gonic/gin"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func assistantHistoryControllerContext(t *testing.T, method, path string, userID int, conversationID int64) (*gin.Context, *httptest.ResponseRecorder) {
	t.Helper()
	response := httptest.NewRecorder()
	c, _ := gin.CreateTestContext(response)
	c.Set("id", userID)
	c.Params = gin.Params{{Key: "id", Value: strconv.FormatInt(conversationID, 10)}}
	c.Request = httptest.NewRequest(method, path, nil)
	return c, response
}

func TestAssistantConversationArchiveControllerIsOwnerOnlyAndUsesEnvelope(t *testing.T) {
	db := setupTokenControllerTestDB(t)
	owner := model.User{Id: 42, Username: "history-owner", Password: "password", AffCode: "history-owner"}
	viewer := model.User{Id: 99, Username: "history-viewer", Password: "password", AffCode: "history-viewer"}
	require.NoError(t, db.Create(&owner).Error)
	require.NoError(t, db.Create(&viewer).Error)
	conversation := &model.AssistantConversation{
		UserId:             owner.Id,
		Title:              "owner conversation",
		LastMessagePreview: "safe preview",
		CreatedAt:          1,
		UpdatedAt:          1,
	}
	require.NoError(t, db.Create(conversation).Error)

	ownerContext, ownerResponse := assistantHistoryControllerContext(
		t,
		http.MethodPost,
		"/api/assistant/conversations/1/archive",
		conversation.UserId,
		conversation.Id,
	)
	ArchiveAssistantConversation(ownerContext)
	assert.Equal(t, http.StatusOK, ownerResponse.Code)
	var ownerEnvelope struct {
		Success bool `json:"success"`
		Data    struct {
			ID         int64 `json:"id"`
			Archived   bool  `json:"archived"`
			ArchivedAt int64 `json:"archived_at"`
		} `json:"data"`
	}
	require.NoError(t, json.Unmarshal(ownerResponse.Body.Bytes(), &ownerEnvelope))
	assert.True(t, ownerEnvelope.Success)
	assert.Equal(t, conversation.Id, ownerEnvelope.Data.ID)
	assert.True(t, ownerEnvelope.Data.Archived)
	assert.Positive(t, ownerEnvelope.Data.ArchivedAt)

	var stored model.AssistantConversation
	require.NoError(t, db.First(&stored, conversation.Id).Error)
	assert.Positive(t, stored.ArchivedAt)

	// A higher-level viewer can read history, but cannot mutate this row.
	viewerContext, viewerResponse := assistantHistoryControllerContext(
		t,
		http.MethodPost,
		"/api/assistant/conversations/1/unarchive",
		99,
		conversation.Id,
	)
	UnarchiveAssistantConversation(viewerContext)
	assert.Equal(t, http.StatusNotFound, viewerResponse.Code)
	var viewerEnvelope struct {
		Success bool   `json:"success"`
		Code    string `json:"code"`
	}
	require.NoError(t, json.Unmarshal(viewerResponse.Body.Bytes(), &viewerEnvelope))
	assert.False(t, viewerEnvelope.Success)
	assert.Equal(t, "ASSISTANT_HISTORY_NOT_FOUND", viewerEnvelope.Code)

	ownerContext, ownerResponse = assistantHistoryControllerContext(
		t,
		http.MethodPost,
		"/api/assistant/conversations/1/unarchive",
		conversation.UserId,
		conversation.Id,
	)
	UnarchiveAssistantConversation(ownerContext)
	assert.Equal(t, http.StatusOK, ownerResponse.Code)
	assert.Contains(t, ownerResponse.Body.String(), `"archived":false`)
}

func TestListAssistantConversationsControllerFiltersArchivedExplicitly(t *testing.T) {
	db := setupTokenControllerTestDB(t)
	owner := model.User{Id: 42, Username: "history-list-owner", Password: "password", AffCode: "history-list-owner"}
	require.NoError(t, db.Create(&owner).Error)
	active := &model.AssistantConversation{
		UserId:             owner.Id,
		Title:              "active",
		LastMessagePreview: "active preview",
		CreatedAt:          1,
		UpdatedAt:          2,
	}
	archived := &model.AssistantConversation{
		UserId:             owner.Id,
		Title:              "archived",
		LastMessagePreview: "archived preview",
		CreatedAt:          1,
		UpdatedAt:          1,
		ArchivedAt:         3,
	}
	require.NoError(t, db.Create(active).Error)
	require.NoError(t, db.Create(archived).Error)
	require.NoError(t, model.RecordAssistantConversationTurn(owner.Id, active.Id, "active question", "active answer"))
	require.NoError(t, model.RecordAssistantConversationTurn(owner.Id, archived.Id, "archived question", "archived answer"))

	listContext, listResponse := assistantHistoryControllerContext(
		t,
		http.MethodGet,
		"/api/assistant/conversations",
		owner.Id,
		0,
	)
	ListAssistantConversations(listContext)
	assert.Equal(t, http.StatusOK, listResponse.Code)
	var activeEnvelope struct {
		Data struct {
			Conversations []model.AssistantConversationView `json:"conversations"`
		} `json:"data"`
	}
	require.NoError(t, json.Unmarshal(listResponse.Body.Bytes(), &activeEnvelope))
	require.Len(t, activeEnvelope.Data.Conversations, 1)
	assert.Equal(t, active.Id, activeEnvelope.Data.Conversations[0].Id)

	listContext, listResponse = assistantHistoryControllerContext(
		t,
		http.MethodGet,
		"/api/assistant/conversations?archived=true",
		owner.Id,
		0,
	)
	ListAssistantConversations(listContext)
	assert.Equal(t, http.StatusOK, listResponse.Code)
	var archivedEnvelope struct {
		Data struct {
			Conversations []model.AssistantConversationView `json:"conversations"`
		} `json:"data"`
	}
	require.NoError(t, json.Unmarshal(listResponse.Body.Bytes(), &archivedEnvelope))
	require.Len(t, archivedEnvelope.Data.Conversations, 1)
	assert.Equal(t, archived.Id, archivedEnvelope.Data.Conversations[0].Id)
}
