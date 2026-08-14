package model

import (
	"fmt"
	"strings"
	"testing"
	"time"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/glebarez/sqlite"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
	"gorm.io/gorm"
)

func setupAssistantGiftTestDB(t *testing.T) *gorm.DB {
	t.Helper()
	previousDB := DB
	previousRedis := common.RedisEnabled
	previousQuotaPerUnit := common.QuotaPerUnit
	previousLocalAcceptance := LocalAcceptanceDeveloperAccessEnabled()
	common.RedisEnabled = false
	common.QuotaPerUnit = 500_000
	SetLocalAcceptanceDeveloperAccess(false)
	dsn := fmt.Sprintf("file:%s?mode=memory&cache=shared", strings.ReplaceAll(t.Name(), "/", "_"))
	db, err := gorm.Open(sqlite.Open(dsn), &gorm.Config{})
	require.NoError(t, err)
	DB = db
	require.NoError(t, db.AutoMigrate(&User{}, &TopUp{}, &AssistantNewUserGift{}, &AssistantGiftRiskMemory{}))
	t.Cleanup(func() {
		DB = previousDB
		common.RedisEnabled = previousRedis
		common.QuotaPerUnit = previousQuotaPerUnit
		SetLocalAcceptanceDeveloperAccess(previousLocalAcceptance)
	})
	return db
}

func newAssistantGiftUser(t *testing.T, db *gorm.DB, username, email string) User {
	t.Helper()
	level := TrustLevelMinUser
	user := User{
		Username: username, Email: email, Role: common.RoleCommonUser,
		Status: common.UserStatusEnabled, CreatedAt: time.Now().Add(-24 * time.Hour).Unix(),
		TrustLevelOverride: &level, AffCode: username + "-aff",
	}
	require.NoError(t, db.Create(&user).Error)
	return user
}

func TestAssistantNewUserGiftIsOneTimeAndClaimIsIdempotent(t *testing.T) {
	db := setupAssistantGiftTestDB(t)
	user := newAssistantGiftUser(t, db, "gift-user", "gift@example.com")

	gift, created, err := DecideAssistantNewUserGift(user.Id, 7, 525, "Clear and constructive project details.", 2, 40, "198.51.100.10")
	require.NoError(t, err)
	assert.True(t, created)
	assert.Equal(t, AssistantGiftOffered, gift.Status)
	assert.Equal(t, 2_625_000, gift.Quota)

	again, created, err := DecideAssistantNewUserGift(user.Id, 8, 1000, "A retry must not replace the first decision.", 3, 60, "198.51.100.11")
	require.NoError(t, err)
	assert.False(t, created)
	assert.Equal(t, gift.Id, again.Id)
	assert.Equal(t, 525, again.AmountCents)

	claimed, alreadyClaimed, err := ClaimAssistantNewUserGift(user.Id)
	require.NoError(t, err)
	assert.False(t, alreadyClaimed)
	assert.Equal(t, AssistantGiftClaimed, claimed.Status)
	var stored User
	require.NoError(t, db.First(&stored, user.Id).Error)
	assert.Equal(t, gift.Quota, stored.Quota)

	_, alreadyClaimed, err = ClaimAssistantNewUserGift(user.Id)
	require.NoError(t, err)
	assert.True(t, alreadyClaimed)
	require.NoError(t, db.First(&stored, user.Id).Error)
	assert.Equal(t, gift.Quota, stored.Quota)
}

func TestAssistantGiftOpportunitySurvivesL1AndOlderAccounts(t *testing.T) {
	db := setupAssistantGiftTestDB(t)
	levelOne := TrustLevelMinUser + 1
	user := newAssistantGiftUser(t, db, "l1-gift-user", "l1-gift@example.com")
	require.NoError(t, db.Model(&User{}).Where("id = ?", user.Id).Updates(map[string]any{
		"created_at":           time.Now().Add(-365 * 24 * time.Hour).Unix(),
		"trust_level_override": levelOne,
	}).Error)

	gift, created, err := DecideAssistantNewUserGift(user.Id, 9, 100, "An L1 user with an unused welcome-gift opportunity.", 2, 52, "198.51.100.22")
	require.NoError(t, err)
	assert.True(t, created)
	assert.Equal(t, AssistantGiftOffered, gift.Status)
}

func TestAssistantNewUserGiftRejectsIneligibleOrShallowDecisions(t *testing.T) {
	db := setupAssistantGiftTestDB(t)
	user := newAssistantGiftUser(t, db, "shallow-gift-user", "shallow@example.com")
	_, _, err := DecideAssistantNewUserGift(user.Id, 0, 100, "Too early.", 1, 100, "198.51.100.20")
	assert.ErrorIs(t, err, ErrAssistantGiftInvalid)
	assert.Equal(t, "insufficient_conversation", AssistantGiftErrorCode(err))

	concise := newAssistantGiftUser(t, db, "concise-gift-user", "concise@example.com")
	// Two substantive turns are sufficient even when the language uses fewer
	// than the old, arbitrary 24-rune aggregate threshold.
	gift, created, err := DecideAssistantNewUserGift(concise.Id, 1, 100, "软件开发与编程辅助。", 2, 23, "198.51.100.23")
	require.NoError(t, err)
	assert.True(t, created)
	assert.Equal(t, AssistantGiftOffered, gift.Status)

	disposable := newAssistantGiftUser(t, db, "disposable-gift-user", "disposable@mailinator.com")
	_, _, err = DecideAssistantNewUserGift(disposable.Id, 0, 100, "Disposable accounts are not rewarded.", 2, 40, "198.51.100.21")
	assert.ErrorIs(t, err, ErrAssistantGiftIneligible)
	assert.Equal(t, "account_not_eligible", AssistantGiftErrorCode(err))

	zero, created, err := DecideAssistantNewUserGift(user.Id, 0, 0, "No gift was earned in this conversation.", 2, 40, "198.51.100.20")
	require.NoError(t, err)
	assert.True(t, created)
	assert.Equal(t, AssistantGiftDeclined, zero.Status)
	_, _, err = ClaimAssistantNewUserGift(user.Id)
	assert.ErrorIs(t, err, ErrAssistantGiftUnavailable)
}

func TestAssistantGiftGlobalRiskMemoryBlocksAliasesAndBulkNetworks(t *testing.T) {
	db := setupAssistantGiftTestDB(t)
	first := newAssistantGiftUser(t, db, "gift-alias-first", "first.last+one@gmail.com")
	alias := newAssistantGiftUser(t, db, "gift-alias-second", "firstlast+two@googlemail.com")

	_, _, err := DecideAssistantNewUserGift(first.Id, 1, 100, "A legitimate first conversation.", 2, 40, "203.0.113.10")
	require.NoError(t, err)
	_, _, err = DecideAssistantNewUserGift(alias.Id, 2, 100, "The same mailbox under another alias.", 2, 40, "203.0.113.11")
	assert.ErrorIs(t, err, ErrAssistantGiftAbuse)
	assert.Equal(t, "identity_already_used", AssistantGiftErrorCode(err))

	for index := 0; index < assistantGiftIPLimit; index++ {
		user := newAssistantGiftUser(t, db, fmt.Sprintf("gift-network-%d", index), fmt.Sprintf("network-%d@example.com", index))
		_, _, err = DecideAssistantNewUserGift(user.Id, int64(index+10), 100, "A distinct account sharing one network.", 2, 40, "203.0.113.20")
		require.NoError(t, err)
	}
	blocked := newAssistantGiftUser(t, db, "gift-network-blocked", "network-blocked@example.com")
	_, _, err = DecideAssistantNewUserGift(blocked.Id, 20, 100, "A fourth decision from one network.", 2, 40, "203.0.113.20")
	assert.ErrorIs(t, err, ErrAssistantGiftAbuse)
	assert.Equal(t, "network_limit_reached", AssistantGiftErrorCode(err))

	var memories []AssistantGiftRiskMemory
	require.NoError(t, db.Find(&memories).Error)
	for _, memory := range memories {
		assert.Len(t, memory.KeyHash, 64)
		assert.NotContains(t, memory.KeyHash, "example.com")
		assert.NotContains(t, memory.KeyHash, "203.0.113")
	}
}
