package controller

import (
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/LIghtJUNction/api.lmm.best/model"
	"github.com/gin-gonic/gin"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func assistantUserProfileRequest(t *testing.T, method, path, body string, role int) *httptest.ResponseRecorder {
	t.Helper()
	recorder := httptest.NewRecorder()
	c, _ := gin.CreateTestContext(recorder)
	c.Request = httptest.NewRequest(method, path, strings.NewReader(body))
	c.Request.Header.Set("Content-Type", "application/json")
	c.Params = gin.Params{{Key: "id", Value: "41"}}
	c.Set("id", 99)
	c.Set("role", role)
	c.Set("username", "profile-admin")
	if method == http.MethodGet {
		AdminGetAssistantUserProfile(c)
	} else {
		AdminUpdateAssistantUserProfile(c)
	}
	return recorder
}

func TestAdminAssistantUserProfileIsAdminOnlyAndRedactsSecrets(t *testing.T) {
	db := setupManageUserTestDB(t)
	require.NoError(t, db.AutoMigrate(&model.AssistantUserProfile{}))
	target := &model.User{Id: 41, Username: "profile-target", Password: "password", Role: common.RoleCommonUser, Status: common.UserStatusEnabled, Group: "default", AffCode: "profile-target-41"}
	require.NoError(t, db.Create(target).Error)
	admin := &model.User{Id: 99, Username: "profile-admin", Password: "password", Role: common.RoleRootUser, Status: common.UserStatusEnabled, Group: "default", AffCode: "profile-admin-99"}
	require.NoError(t, db.Create(admin).Error)

	denied := assistantUserProfileRequest(t, http.MethodGet, "/api/user/41/assistant-profile", "", common.RoleCommonUser)
	assert.Equal(t, http.StatusForbidden, denied.Code)

	updated := assistantUserProfileRequest(t, http.MethodPut, "/api/user/41/assistant-profile", `{"profile_key":"guided_buyer","tags":["new user","needs setup"],"strategy":"Never reveal api_key: sk-hidden-value; guide one step at a time.","enabled":true}`, common.RoleRootUser)
	assert.Equal(t, http.StatusOK, updated.Code)
	assert.NotContains(t, updated.Body.String(), "sk-hidden-value")
	assert.Contains(t, updated.Body.String(), "needs setup")

	loaded := assistantUserProfileRequest(t, http.MethodGet, "/api/user/41/assistant-profile", "", common.RoleRootUser)
	assert.Equal(t, http.StatusOK, loaded.Code)
	assert.Contains(t, loaded.Body.String(), "guided_buyer")
	assert.NotContains(t, loaded.Body.String(), "sk-hidden-value")
}
