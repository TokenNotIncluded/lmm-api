package model

const legacyOpenSourceBountyParticipantIndex = "idx_open_source_bounty_participant"

// The original unique project/participant index prevented contributors from
// retaining a rejected attempt and starting a later retry.
func migrateOpenSourceBountyChallengeRetryIndex() error {
	if !DB.Migrator().HasTable(&OpenSourceBountyChallenge{}) ||
		!DB.Migrator().HasIndex(&OpenSourceBountyChallenge{}, legacyOpenSourceBountyParticipantIndex) {
		return nil
	}
	return DB.Migrator().DropIndex(&OpenSourceBountyChallenge{}, legacyOpenSourceBountyParticipantIndex)
}

// migrateOpenSourceBountyMCPTokenAuthVersions binds pre-existing tokens to the
// owner's current authentication version. Tokens belonging to accounts that
// are already below the developer-access boundary are removed instead of
// becoming eligible to revive after a later upgrade.
func migrateOpenSourceBountyMCPTokenAuthVersions() error {
	if !DB.Migrator().HasTable(&OpenSourceBountyMCPToken{}) {
		return nil
	}
	var tokens []OpenSourceBountyMCPToken
	if err := DB.Where("user_auth_version = 0").Find(&tokens).Error; err != nil {
		return err
	}
	for _, token := range tokens {
		authVersion, err := openSourceBountyDeveloperAuthVersion(token.UserId)
		if err != nil {
			if OpenSourceBountyErrorCode(err) != openSourceBountyDeveloperAccessRequiredCode {
				return err
			}
			if err := DB.Delete(&token).Error; err != nil {
				return err
			}
			continue
		}
		if err := DB.Model(&OpenSourceBountyMCPToken{}).
			Where("id = ? AND user_auth_version = 0", token.Id).
			Update("user_auth_version", authVersion).Error; err != nil {
			return err
		}
	}
	return nil
}
