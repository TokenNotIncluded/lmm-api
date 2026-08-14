package router

import (
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/LIghtJUNction/api.lmm.best/model"
	"github.com/gin-gonic/gin"
	"github.com/glebarez/sqlite"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
	"gorm.io/gorm"
)

func setupOpenSourceBountyAccessRouterTest(t *testing.T) *gorm.DB {
	t.Helper()
	previousDB := model.DB
	previousRedisEnabled := common.RedisEnabled
	previousMainDatabaseType, previousLogDatabaseType := common.MainDatabaseType(), common.LogDatabaseType()
	common.RedisEnabled = false
	common.SetDatabaseTypes(common.DatabaseTypeSQLite, previousLogDatabaseType)
	dsn := fmt.Sprintf("file:%s?mode=memory&cache=shared", strings.ReplaceAll(t.Name(), "/", "_"))
	db, err := gorm.Open(sqlite.Open(dsn), &gorm.Config{})
	require.NoError(t, err)
	model.DB = db
	require.NoError(t, db.AutoMigrate(
		&model.User{}, &model.OpenSourceBountyProject{}, &model.OpenSourceBountyChallenge{},
		&model.OpenSourceBountyLedger{}, &model.OpenSourceBountyDispute{},
	))
	t.Cleanup(func() {
		model.DB = previousDB
		common.RedisEnabled = previousRedisEnabled
		common.SetDatabaseTypes(previousMainDatabaseType, previousLogDatabaseType)
		sqlDB, sqlErr := db.DB()
		if sqlErr == nil {
			_ = sqlDB.Close()
		}
	})
	return db
}

func TestOpenSourceBountyL0CanBrowsePublicListDetailAndConfig(t *testing.T) {
	db := setupOpenSourceBountyAccessRouterTest(t)
	levelZero := model.TrustLevelMinUser
	pat := "bounty-l0-public-pat"
	viewer := model.User{
		Username: "bounty-public-l0", Password: "password", AffCode: "bounty-public-l0", Group: "default",
		Role: common.RoleCommonUser, Status: common.UserStatusEnabled, TrustLevelOverride: &levelZero, AccessToken: &pat,
	}
	owner := model.User{
		Username: "bounty-public-owner", Password: "password", AffCode: "bounty-public-owner", Group: "default",
		Role: common.RoleAdminUser, Status: common.UserStatusEnabled,
	}
	require.NoError(t, db.Create(&viewer).Error)
	require.NoError(t, db.Create(&owner).Error)
	project := model.OpenSourceBountyProject{
		OwnerUserId: owner.Id, RepositoryUrl: "https://github.com/example/public", Title: "Public bounty",
		Description: "A public bounty description", Rules: "Public verification rules", RewardQuota: 100,
		NetRewardQuota: 99, RewardSlots: 1, EscrowQuota: 99, Status: model.OpenSourceBountyStatusPublished,
		CreatedAt: 1, UpdatedAt: 1, PublishedAt: 1,
	}
	require.NoError(t, db.Create(&project).Error)

	gin.SetMode(gin.TestMode)
	engine := gin.New()
	SetApiRouter(engine)
	for _, path := range []string{
		"/api/open-source-bounties",
		fmt.Sprintf("/api/open-source-bounties/projects/%d", project.Id),
		"/api/open-source-bounties/config",
	} {
		response := httptest.NewRecorder()
		request := httptest.NewRequest(http.MethodGet, path, nil)
		request.Header.Set("Authorization", "Bearer "+pat)
		engine.ServeHTTP(response, request)
		assert.Equal(t, http.StatusOK, response.Code, path)
		assert.Contains(t, response.Body.String(), `"success":true`, path)
		assert.NotContains(t, response.Body.String(), `"message":"Not Found"`, path)
	}
}

func TestOpenSourceBountyPrivateAccessRequiresL1ForReadsAndWrites(t *testing.T) {
	db := setupOpenSourceBountyAccessRouterTest(t)
	levelZero, levelOne := model.TrustLevelMinUser, model.TrustLevelMinUser+1
	users := []model.User{
		{Username: "bounty-router-l0", Password: "password", AffCode: "bounty-router-l0", Role: common.RoleCommonUser, Status: common.UserStatusEnabled, TrustLevelOverride: &levelZero},
		{Username: "bounty-router-l1", Password: "password", AffCode: "bounty-router-l1", Role: common.RoleCommonUser, Status: common.UserStatusEnabled, TrustLevelOverride: &levelOne},
		{Username: "bounty-router-admin", Password: "password", AffCode: "bounty-router-admin", Role: common.RoleAdminUser, Status: common.UserStatusEnabled},
	}
	for index := range users {
		require.NoError(t, db.Create(&users[index]).Error)
	}

	for _, test := range []struct {
		name       string
		method     string
		path       string
		user       model.User
		wantStatus int
		wantEmpty  bool
	}{
		{name: "L0 private read is redacted", method: http.MethodGet, path: "/api/open-source-bounties/mine", user: users[0], wantStatus: http.StatusOK, wantEmpty: true},
		{name: "L0 write is hidden", method: http.MethodPost, path: "/api/open-source-bounties", user: users[0], wantStatus: http.StatusNotFound},
		{name: "L1 private read", method: http.MethodGet, path: "/api/open-source-bounties/mine", user: users[1], wantStatus: http.StatusNoContent},
		{name: "L1 write", method: http.MethodPost, path: "/api/open-source-bounties", user: users[1], wantStatus: http.StatusNoContent},
		{name: "administrator private read", method: http.MethodGet, path: "/api/open-source-bounties/mine", user: users[2], wantStatus: http.StatusNoContent},
		{name: "administrator write", method: http.MethodPost, path: "/api/open-source-bounties", user: users[2], wantStatus: http.StatusNoContent},
	} {
		t.Run(test.name, func(t *testing.T) {
			engine := gin.New()
			engine.Use(func(c *gin.Context) {
				c.Set("id", test.user.Id)
				c.Next()
			})
			engine.Use(requireOpenSourceBountyDeveloperAccess())
			engine.Handle(test.method, test.path, func(c *gin.Context) { c.Status(http.StatusNoContent) })
			response := httptest.NewRecorder()
			engine.ServeHTTP(response, httptest.NewRequest(test.method, test.path, nil))
			assert.Equal(t, test.wantStatus, response.Code)
			if test.wantStatus == http.StatusNotFound {
				assert.JSONEq(t, `{"message":"Not Found"}`, response.Body.String())
			}
			if test.wantEmpty {
				assert.JSONEq(t, `{"success":true,"message":"","data":[]}`, response.Body.String())
			}
		})
	}
}
