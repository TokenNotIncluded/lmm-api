package middleware

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

func setupPaymentAccessTestDB(t *testing.T) *gorm.DB {
	t.Helper()
	previousDB := model.DB
	previousRedisEnabled := common.RedisEnabled
	common.RedisEnabled = false
	db, err := gorm.Open(sqlite.Open(fmt.Sprintf("file:%s?mode=memory&cache=shared", strings.ReplaceAll(t.Name(), "/", "_"))), &gorm.Config{})
	require.NoError(t, err)
	require.NoError(t, db.AutoMigrate(&model.User{}))
	model.DB = db
	t.Cleanup(func() {
		model.DB = previousDB
		common.RedisEnabled = previousRedisEnabled
		sqlDB, _ := db.DB()
		_ = sqlDB.Close()
	})
	return db
}

func TestPaymentAccessGateRejectsRestrictedUsers(t *testing.T) {
	db := setupPaymentAccessTestDB(t)
	user := model.User{
		Username:                "restricted-payment",
		Password:                "password123",
		PaymentRestrictionFlags: model.PaymentRestrictionLinuxDOHighScore,
	}
	require.NoError(t, db.Create(&user).Error)

	gin.SetMode(gin.TestMode)
	router := gin.New()
	router.Use(func(c *gin.Context) {
		c.Set("id", user.Id)
		c.Next()
	})
	router.POST("/pay", PaymentAccessGate(), func(c *gin.Context) {
		c.Status(http.StatusNoContent)
	})

	response := httptest.NewRecorder()
	router.ServeHTTP(response, httptest.NewRequest(http.MethodPost, "/pay", nil))

	assert.Equal(t, http.StatusForbidden, response.Code)
	assert.JSONEq(t, `{"success":false,"code":"PAYMENT_UNAVAILABLE","message":"Payment is unavailable for this account."}`, response.Body.String())
}

func TestPaymentAccessGateAllowsOrdinaryUsers(t *testing.T) {
	db := setupPaymentAccessTestDB(t)
	user := model.User{Username: "ordinary-payment", Password: "password123", Email: "member@example.com"}
	require.NoError(t, db.Create(&user).Error)

	gin.SetMode(gin.TestMode)
	router := gin.New()
	router.Use(func(c *gin.Context) {
		c.Set("id", user.Id)
		c.Next()
	})
	router.POST("/pay", PaymentAccessGate(), func(c *gin.Context) {
		c.Status(http.StatusNoContent)
	})

	response := httptest.NewRecorder()
	router.ServeHTTP(response, httptest.NewRequest(http.MethodPost, "/pay", nil))
	assert.Equal(t, http.StatusNoContent, response.Code)
}

func TestPaymentMethodAccessGateLetsControllerApplyPerMethodRule(t *testing.T) {
	db := setupPaymentAccessTestDB(t)
	user := model.User{
		Username:                "audience-payment",
		Password:                "password123",
		PaymentRestrictionFlags: model.PaymentRestrictionLinuxDOHighScore,
	}
	require.NoError(t, db.Create(&user).Error)

	gin.SetMode(gin.TestMode)
	router := gin.New()
	router.Use(func(c *gin.Context) {
		c.Set("id", user.Id)
		c.Next()
	})
	router.POST("/pay", PaymentMethodAccessGate(), func(c *gin.Context) {
		_, loaded := c.Get("payment_user")
		assert.True(t, loaded)
		c.Status(http.StatusNoContent)
	})

	response := httptest.NewRecorder()
	router.ServeHTTP(response, httptest.NewRequest(http.MethodPost, "/pay", nil))
	assert.Equal(t, http.StatusNoContent, response.Code)
}
