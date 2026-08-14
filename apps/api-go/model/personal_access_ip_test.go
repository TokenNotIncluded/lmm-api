package model

import (
	"fmt"
	"strings"
	"testing"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/glebarez/sqlite"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
	"gorm.io/gorm"
)

func TestNormalizePersonalAccessIPRejectsNonPublicAddresses(t *testing.T) {
	for _, input := range []string{
		"",
		"127.0.0.1",
		"10.0.0.1",
		"100.64.0.1",
		"192.0.2.10",
		"198.51.100.10",
		"203.0.113.10",
		"2001:db8::10",
		"::1",
	} {
		_, err := NormalizePersonalAccessIP(input)
		assert.ErrorIs(t, err, ErrInvalidPersonalAccessIP, input)
	}
}

func TestNormalizePersonalAccessIPCanonicalizesGlobalAddress(t *testing.T) {
	assert.Equal(t, "8.8.8.8", mustNormalizePersonalAccessIP(t, " 8.8.8.8 "))
	assert.Equal(t, "2001:4860:4860::8888", mustNormalizePersonalAccessIP(t, "2001:4860:4860:0:0:0:0:8888"))
}

func mustNormalizePersonalAccessIP(t *testing.T, input string) string {
	t.Helper()
	ip, err := NormalizePersonalAccessIP(input)
	require.NoError(t, err)
	return ip
}

func TestPersonalAccessIPRequiresL1AndEnforcesOnePerUser(t *testing.T) {
	previousDB := DB
	previousRedis := common.RedisEnabled
	common.RedisEnabled = false
	db, err := gorm.Open(sqlite.Open(fmt.Sprintf("file:%s?mode=memory&cache=shared", strings.ReplaceAll(t.Name(), "/", "_"))), &gorm.Config{})
	require.NoError(t, err)
	DB = db
	require.NoError(t, db.AutoMigrate(&User{}, &TopUp{}, &PersonalAccessIP{}))
	t.Cleanup(func() {
		DB = previousDB
		common.RedisEnabled = previousRedis
		sqlDB, _ := db.DB()
		_ = sqlDB.Close()
	})

	lowLevel := TrustLevelMinUser
	highLevel := PersonalAccessIPMinTrustLevel
	lowUser := User{Username: "ip-low", Password: "password", AffCode: "ip-low-aff", Role: common.RoleCommonUser, Status: common.UserStatusEnabled, TrustLevelOverride: &lowLevel}
	highUser := User{Username: "ip-high", Password: "password", AffCode: "ip-high-aff", Role: common.RoleCommonUser, Status: common.UserStatusEnabled, TrustLevelOverride: &highLevel}
	require.NoError(t, db.Create(&lowUser).Error)
	require.NoError(t, db.Create(&highUser).Error)

	_, err = SetPersonalAccessIP(&lowUser, "8.8.8.8")
	assert.ErrorIs(t, err, ErrPersonalAccessIPNotEligible)
	record, err := SetPersonalAccessIP(&highUser, "8.8.8.8")
	require.NoError(t, err)
	assert.Equal(t, "8.8.8.8", record.IP)
	record, err = SetPersonalAccessIP(&highUser, "1.1.1.1")
	require.NoError(t, err)
	assert.Equal(t, "1.1.1.1", record.IP)
	assert.False(t, mustPersonalAccessIPAllowedForUser(t, highUser.Id, "8.8.8.8"))
	assert.True(t, mustPersonalAccessIPAllowedForUser(t, highUser.Id, "1.1.1.1"))
	// A stale record must not regain access when the account is below L1.
	require.NoError(t, db.Create(&PersonalAccessIP{UserId: lowUser.Id, IP: "1.1.1.1"}).Error)
	assert.False(t, mustPersonalAccessIPAllowedForUser(t, lowUser.Id, "1.1.1.1"))

	otherLevel := PersonalAccessIPMinTrustLevel
	otherUser := User{Username: "ip-other", Password: "password", AffCode: "ip-other-aff", Role: common.RoleCommonUser, Status: common.UserStatusEnabled, TrustLevelOverride: &otherLevel}
	require.NoError(t, db.Create(&otherUser).Error)
	_, err = SetPersonalAccessIP(&otherUser, "1.1.1.1")
	require.NoError(t, err)
	assert.True(t, mustPersonalAccessIPAllowedForUser(t, otherUser.Id, "1.1.1.1"))
}

func mustPersonalAccessIPAllowedForUser(t *testing.T, userID int, input string) bool {
	t.Helper()
	allowed, err := IsPersonalAccessIPAllowedForUser(userID, input)
	require.NoError(t, err)
	return allowed
}
