package controller

import (
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"testing"

	"github.com/QuantumNous/new-api/common"
	"github.com/QuantumNous/new-api/constant"
	"github.com/QuantumNous/new-api/model"
	"github.com/QuantumNous/new-api/setting/operation_setting"
	"github.com/gin-gonic/gin"
	"github.com/glebarez/sqlite"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
	"gorm.io/gorm"
)

func setupPostSetupTestDB(t *testing.T) *gorm.DB {
	t.Helper()
	gin.SetMode(gin.TestMode)

	previousDB := model.DB
	previousSetup := constant.IsSetup()
	previousOptionMap := common.OptionMap
	previousMainDatabaseType := common.MainDatabaseType()
	previousLogDatabaseType := common.LogDatabaseType()
	previousSelfUseMode := operation_setting.SelfUseModeEnabled
	previousDemoSite := operation_setting.DemoSiteEnabled

	common.SetDatabaseTypes(common.DatabaseTypeSQLite, common.DatabaseTypeSQLite)
	constant.SetSetup(false)
	common.OptionMap = map[string]string{}
	operation_setting.SelfUseModeEnabled = false
	operation_setting.DemoSiteEnabled = false

	dsn := fmt.Sprintf("file:%s?mode=memory&cache=shared", strings.ReplaceAll(t.Name(), "/", "_"))
	db, err := gorm.Open(sqlite.Open(dsn), &gorm.Config{})
	require.NoError(t, err)
	require.NoError(t, db.AutoMigrate(&model.User{}, &model.Option{}, &model.Setup{}))
	model.DB = db

	t.Cleanup(func() {
		model.DB = previousDB
		constant.SetSetup(previousSetup)
		common.OptionMap = previousOptionMap
		common.SetDatabaseTypes(previousMainDatabaseType, previousLogDatabaseType)
		operation_setting.SelfUseModeEnabled = previousSelfUseMode
		operation_setting.DemoSiteEnabled = previousDemoSite
		if sqlDB, err := db.DB(); err == nil {
			_ = sqlDB.Close()
		}
	})

	return db
}

func performPostSetupRequest(body string) *httptest.ResponseRecorder {
	recorder := httptest.NewRecorder()
	c, _ := gin.CreateTestContext(recorder)
	c.Request = httptest.NewRequest(http.MethodPost, "/api/setup", strings.NewReader(body))
	c.Request.Header.Set("Content-Type", "application/json")
	PostSetup(c)
	return recorder
}

func TestPostSetupConcurrentRequestsCreateSingleRoot(t *testing.T) {
	db := setupPostSetupTestDB(t)

	const requestCount = 16
	start := make(chan struct{})
	responses := make(chan string, requestCount)
	var wg sync.WaitGroup

	for i := 0; i < requestCount; i++ {
		i := i
		wg.Add(1)
		go func() {
			defer wg.Done()
			<-start
			body := fmt.Sprintf(`{"username":"root%d","password":"Password123","confirmPassword":"Password123","SelfUseModeEnabled":true,"DemoSiteEnabled":false}`, i)
			responses <- performPostSetupRequest(body).Body.String()
		}()
	}

	close(start)
	wg.Wait()
	close(responses)

	successCount := 0
	for response := range responses {
		if strings.Contains(response, `"success":true`) {
			successCount++
		}
	}

	var rootCount int64
	require.NoError(t, db.Model(&model.User{}).Where("role = ?", common.RoleRootUser).Count(&rootCount).Error)
	var setupCount int64
	require.NoError(t, db.Model(&model.Setup{}).Count(&setupCount).Error)
	var setup model.Setup
	require.NoError(t, db.First(&setup).Error)

	assert.Equal(t, 1, successCount)
	assert.Equal(t, int64(1), rootCount)
	assert.Equal(t, int64(1), setupCount)
	assert.Equal(t, model.SetupSingletonID, setup.ID)
}
