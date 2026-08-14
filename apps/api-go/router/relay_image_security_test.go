package router

import (
	"net/http"
	"net/http/httptest"
	"net/url"
	"testing"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/LIghtJUNction/api.lmm.best/model"
	"github.com/LIghtJUNction/api.lmm.best/relay"
	"github.com/LIghtJUNction/api.lmm.best/service"
	"github.com/LIghtJUNction/api.lmm.best/setting/system_setting"
	"github.com/gin-gonic/gin"
	"github.com/glebarez/sqlite"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
	"gorm.io/gorm"
)

func setupRelayImageSecurityTest(t *testing.T) {
	t.Helper()
	previousDB := model.DB
	previousDatabaseType := common.MainDatabaseType()
	previousMemoryCache := common.MemoryCacheEnabled
	previousFetchSetting := *system_setting.GetFetchSetting()

	db, err := gorm.Open(sqlite.Open(t.TempDir()+"/relay-image.db"), &gorm.Config{})
	require.NoError(t, err)
	t.Cleanup(func() {
		sqlDB, err := db.DB()
		if err == nil {
			_ = sqlDB.Close()
		}
	})
	model.DB = db
	common.SetMainDatabaseType(common.DatabaseTypeSQLite)
	common.MemoryCacheEnabled = false
	fetchSetting := system_setting.GetFetchSetting()
	fetchSetting.EnableSSRFProtection = false

	require.NoError(t, db.AutoMigrate(&model.Midjourney{}, &model.Channel{}))
	service.InitHttpClient()
	t.Cleanup(func() {
		model.DB = previousDB
		common.SetMainDatabaseType(previousDatabaseType)
		common.MemoryCacheEnabled = previousMemoryCache
		*system_setting.GetFetchSetting() = previousFetchSetting
	})
}

func TestRelayMidjourneyImageIsNotPubliclyReadable(t *testing.T) {
	setupRelayImageSecurityTest(t)

	imageServer := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_, _ = w.Write([]byte("user-a-image"))
	}))
	t.Cleanup(imageServer.Close)
	require.NoError(t, model.DB.Create(&model.Midjourney{
		UserId:    101,
		MjId:      "task-owned-by-a",
		ImageUrl:  imageServer.URL,
		ChannelId: 999,
	}).Error)

	gin.SetMode(gin.TestMode)
	engine := gin.New()
	SetRelayRouter(engine)
	for _, path := range []string{
		"/mj/image/task-owned-by-a",
		"/openai/mj/image/task-owned-by-a",
	} {
		request := httptest.NewRequest(http.MethodGet, path, nil)
		response := httptest.NewRecorder()

		engine.ServeHTTP(response, request)

		assert.Equal(t, http.StatusUnauthorized, response.Code, path)
		assert.Contains(t, response.Body.String(), "midjourney_image_signature_invalid", path)
	}
}

func TestRelayMidjourneyImageRequiresTaskOwnerAndValidSignature(t *testing.T) {
	setupRelayImageSecurityTest(t)

	imageServer := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_, _ = w.Write([]byte("user-a-image"))
	}))
	t.Cleanup(imageServer.Close)
	require.NoError(t, model.DB.Create(&model.Midjourney{
		UserId:    101,
		MjId:      "task-owned-by-a",
		ImageUrl:  imageServer.URL,
		ChannelId: 999,
	}).Error)

	gin.SetMode(gin.TestMode)
	engine := gin.New()
	SetRelayRouter(engine)
	imageURL := relay.BuildMidjourneyImageURL("http://fixture", 202, "task-owned-by-a")
	parsedURL, err := url.Parse(imageURL)
	require.NoError(t, err)
	request := httptest.NewRequest(http.MethodGet, parsedURL.RequestURI(), nil)
	response := httptest.NewRecorder()

	engine.ServeHTTP(response, request)

	assert.Equal(t, http.StatusNotFound, response.Code)
	assert.Contains(t, response.Body.String(), "midjourney_task_not_found")
}

func TestRelayMidjourneyImageAllowsSignedTaskURL(t *testing.T) {
	setupRelayImageSecurityTest(t)

	imageServer := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "image/png")
		_, _ = w.Write([]byte("user-a-image"))
	}))
	t.Cleanup(imageServer.Close)
	require.NoError(t, model.DB.Create(&model.Midjourney{
		UserId:    101,
		MjId:      "task-owned-by-a",
		ImageUrl:  imageServer.URL,
		ChannelId: 999,
	}).Error)

	gin.SetMode(gin.TestMode)
	engine := gin.New()
	SetRelayRouter(engine)
	imageURL := relay.BuildMidjourneyImageURL("http://fixture", 101, "task-owned-by-a")
	parsedURL, err := url.Parse(imageURL)
	require.NoError(t, err)
	for _, path := range []string{
		parsedURL.RequestURI(),
		"/openai" + parsedURL.RequestURI(),
	} {
		request := httptest.NewRequest(http.MethodGet, path, nil)
		response := httptest.NewRecorder()

		engine.ServeHTTP(response, request)

		assert.Equal(t, http.StatusOK, response.Code, path)
		assert.Equal(t, "image/png", response.Header().Get("Content-Type"), path)
		assert.Equal(t, "user-a-image", response.Body.String(), path)
	}
}

func TestRelayMidjourneyImageRejectsTamperedOwner(t *testing.T) {
	setupRelayImageSecurityTest(t)
	require.NoError(t, model.DB.Create(&model.Midjourney{
		UserId:    101,
		MjId:      "task-owned-by-a",
		ImageUrl:  "https://example.com/image.png",
		ChannelId: 999,
	}).Error)

	gin.SetMode(gin.TestMode)
	engine := gin.New()
	SetRelayRouter(engine)
	imageURL := relay.BuildMidjourneyImageURL("http://fixture", 101, "task-owned-by-a")
	parsedURL, err := url.Parse(imageURL)
	require.NoError(t, err)
	query := parsedURL.Query()
	query.Set("uid", "202")
	parsedURL.RawQuery = query.Encode()
	request := httptest.NewRequest(http.MethodGet, parsedURL.RequestURI(), nil)
	response := httptest.NewRecorder()

	engine.ServeHTTP(response, request)

	assert.Equal(t, http.StatusUnauthorized, response.Code)
	assert.Contains(t, response.Body.String(), "midjourney_image_signature_invalid")
}
