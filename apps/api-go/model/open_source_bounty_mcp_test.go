package model

import (
	"fmt"
	"testing"

	"github.com/QuantumNous/new-api/common"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
	"gorm.io/gorm"
)

func TestOpenSourceBountyMCPTokenRotationAndRevocation(t *testing.T) {
	db := setupOpenSourceBountyTestDB(t)
	require.NoError(t, db.AutoMigrate(&OpenSourceBountyMCPToken{}, &OpenSourceBountyMCPConfirmation{}, &OpenSourceBountyMCPOperation{}))
	user := createOpenSourceBountyUser(t, db, "mcp-user", 100, common.RoleCommonUser)

	first, status, err := RotateOpenSourceBountyMCPToken(user.Id)
	require.NoError(t, err)
	assert.NotEmpty(t, first)
	assert.NotContains(t, status.TokenHint, first)
	verifiedUser, err := VerifyOpenSourceBountyMCPToken(first)
	require.NoError(t, err)
	assert.Equal(t, user.Id, verifiedUser)

	second, _, err := RotateOpenSourceBountyMCPToken(user.Id)
	require.NoError(t, err)
	assert.NotEqual(t, first, second)
	_, err = VerifyOpenSourceBountyMCPToken(first)
	assert.Equal(t, "OPEN_SOURCE_BOUNTY_MCP_INVALID_TOKEN", OpenSourceBountyErrorCode(err))
	verifiedUser, err = VerifyOpenSourceBountyMCPToken(second)
	require.NoError(t, err)
	assert.Equal(t, user.Id, verifiedUser)

	tokenStatus, err := GetOpenSourceBountyMCPTokenStatus(user.Id)
	require.NoError(t, err)
	assert.True(t, tokenStatus.Configured)
	require.NoError(t, RevokeOpenSourceBountyMCPToken(user.Id))
	_, err = VerifyOpenSourceBountyMCPToken(second)
	assert.Equal(t, "OPEN_SOURCE_BOUNTY_MCP_INVALID_TOKEN", OpenSourceBountyErrorCode(err))
}

func TestOpenSourceBountyMCPTokenRequiresCurrentDeveloperAccess(t *testing.T) {
	db := setupOpenSourceBountyTestDB(t)
	require.NoError(t, db.AutoMigrate(&OpenSourceBountyMCPToken{}))
	levelZero := TrustLevelMinUser
	l0 := User{
		Username: "mcp-l0", Password: "password", AffCode: "mcp-l0", Role: common.RoleCommonUser,
		Status: common.UserStatusEnabled, TrustLevelOverride: &levelZero,
	}
	require.NoError(t, db.Create(&l0).Error)
	_, _, err := RotateOpenSourceBountyMCPToken(l0.Id)
	assert.Equal(t, "OPEN_SOURCE_BOUNTY_MCP_FORBIDDEN", OpenSourceBountyErrorCode(err))

	l1 := createOpenSourceBountyUser(t, db, "mcp-downgraded", 100, common.RoleCommonUser)
	token, _, err := RotateOpenSourceBountyMCPToken(l1.Id)
	require.NoError(t, err)
	require.NoError(t, db.Model(&User{}).Where("id = ?", l1.Id).Updates(map[string]any{
		"trust_level_override": TrustLevelMinUser,
		"auth_version":         gorm.Expr("auth_version + 1"),
	}).Error)
	_, err = VerifyOpenSourceBountyMCPToken(token)
	assert.Equal(t, "OPEN_SOURCE_BOUNTY_MCP_INVALID_TOKEN", OpenSourceBountyErrorCode(err), "an old MCP token must fail on the first call after downgrade")
	_, err = GetOpenSourceBountyMCPTokenStatus(l1.Id)
	assert.Equal(t, "OPEN_SOURCE_BOUNTY_MCP_FORBIDDEN", OpenSourceBountyErrorCode(err))
	err = RevokeOpenSourceBountyMCPToken(l1.Id)
	assert.Equal(t, "OPEN_SOURCE_BOUNTY_MCP_FORBIDDEN", OpenSourceBountyErrorCode(err))
	require.NoError(t, db.Model(&User{}).Where("id = ?", l1.Id).Update("trust_level_override", TrustLevelMinUser+1).Error)
	_, err = VerifyOpenSourceBountyMCPToken(token)
	assert.Equal(t, "OPEN_SOURCE_BOUNTY_MCP_INVALID_TOKEN", OpenSourceBountyErrorCode(err), "a downgraded token must not revive after L1 is restored")

	admin := createOpenSourceBountyUser(t, db, "mcp-admin", 100, common.RoleAdminUser)
	adminToken, _, err := RotateOpenSourceBountyMCPToken(admin.Id)
	require.NoError(t, err)
	verifiedUser, err := VerifyOpenSourceBountyMCPToken(adminToken)
	require.NoError(t, err)
	assert.Equal(t, admin.Id, verifiedUser, "administrator role remains developer-compatible")
}

func TestOpenSourceBountyMCPTokenAuthVersionMigrationPreservesOnlyCurrentDevelopers(t *testing.T) {
	db := setupOpenSourceBountyTestDB(t)
	require.NoError(t, db.AutoMigrate(&OpenSourceBountyMCPToken{}))
	levelZero, levelOne := TrustLevelMinUser, TrustLevelMinUser+1
	users := []User{
		{Username: "mcp-migration-l0", Password: "password", AffCode: "mcp-migration-l0", Role: common.RoleCommonUser, Status: common.UserStatusEnabled, TrustLevelOverride: &levelZero},
		{Username: "mcp-migration-l1", Password: "password", AffCode: "mcp-migration-l1", Role: common.RoleCommonUser, Status: common.UserStatusEnabled, TrustLevelOverride: &levelOne},
		{Username: "mcp-migration-admin", Password: "password", AffCode: "mcp-migration-admin", Role: common.RoleAdminUser, Status: common.UserStatusEnabled},
	}
	for index := range users {
		require.NoError(t, db.Create(&users[index]).Error)
		require.NoError(t, db.Create(&OpenSourceBountyMCPToken{
			UserId: users[index].Id, TokenHash: fmt.Sprintf("%064d", index+1), TokenHint: fmt.Sprintf("token-%d", index),
			CreatedAt: 1, UpdatedAt: 1,
		}).Error)
	}

	require.NoError(t, migrateOpenSourceBountyMCPTokenAuthVersions())
	var l0Tokens int64
	require.NoError(t, db.Model(&OpenSourceBountyMCPToken{}).Where("user_id = ?", users[0].Id).Count(&l0Tokens).Error)
	assert.Zero(t, l0Tokens)
	for _, user := range users[1:] {
		var token OpenSourceBountyMCPToken
		require.NoError(t, db.Where("user_id = ?", user.Id).First(&token).Error)
		assert.Equal(t, user.AuthVersion, token.UserAuthVersion)
	}
}

func TestOpenSourceBountyMCPConfirmationIsBoundAndSingleUse(t *testing.T) {
	db := setupOpenSourceBountyTestDB(t)
	require.NoError(t, db.AutoMigrate(&OpenSourceBountyMCPConfirmation{}))
	user := createOpenSourceBountyUser(t, db, "mcp-confirm", 100, common.RoleCommonUser)
	payloadHash, err := OpenSourceBountyMCPPayloadHash(map[string]any{"project_id": 7})
	require.NoError(t, err)
	state, err := CreateOpenSourceBountyMCPConfirmation(user.Id, "open_source_bounties.publish", payloadHash)
	require.NoError(t, err)

	wrongHash, err := OpenSourceBountyMCPPayloadHash(map[string]any{"project_id": 8})
	require.NoError(t, err)
	err = ConsumeOpenSourceBountyMCPConfirmation(user.Id, "open_source_bounties.publish", wrongHash, state)
	assert.Equal(t, "OPEN_SOURCE_BOUNTY_MCP_CONFIRMATION_INVALID", OpenSourceBountyErrorCode(err))
	err = ConsumeOpenSourceBountyMCPConfirmation(user.Id, "open_source_bounties.publish", payloadHash, state)
	assert.Equal(t, "OPEN_SOURCE_BOUNTY_MCP_CONFIRMATION_INVALID", OpenSourceBountyErrorCode(err))
}
