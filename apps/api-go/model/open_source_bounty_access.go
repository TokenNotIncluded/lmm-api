package model

import (
	"errors"

	"github.com/QuantumNous/new-api/common"
	"gorm.io/gorm"
)

const openSourceBountyDeveloperAccessRequiredCode = "OPEN_SOURCE_BOUNTY_DEVELOPER_ACCESS_REQUIRED"

func openSourceBountyDeveloperAccessRequired() error {
	return bountyError(openSourceBountyDeveloperAccessRequiredCode, "developer access is required for this open-source bounty operation")
}

func openSourceBountyDeveloper(userId int) (*User, error) {
	if userId <= 0 {
		return nil, openSourceBountyDeveloperAccessRequired()
	}
	var user User
	err := DB.Select("id", "role", "status", "trust_level_override", "console_activated_at", "auth_version").
		Where("id = ? AND deleted_at IS NULL", userId).
		First(&user).Error
	if errors.Is(err, gorm.ErrRecordNotFound) {
		return nil, openSourceBountyDeveloperAccessRequired()
	}
	if err != nil {
		return nil, err
	}
	if user.Status != common.UserStatusEnabled {
		return nil, openSourceBountyDeveloperAccessRequired()
	}
	access, err := GetDeveloperAccessStateForUser(&user)
	if err != nil {
		return nil, err
	}
	if !access.Granted {
		return nil, openSourceBountyDeveloperAccessRequired()
	}
	return &user, nil
}

// RequireOpenSourceBountyDeveloperAccess revalidates the current durable user
// state instead of trusting a session or token snapshot. This keeps every
// private bounty boundary fail-closed immediately after an L1-to-L0 downgrade.
func RequireOpenSourceBountyDeveloperAccess(userId int) error {
	_, err := openSourceBountyDeveloper(userId)
	return err
}

func openSourceBountyDeveloperAuthVersion(userId int) (int64, error) {
	user, err := openSourceBountyDeveloper(userId)
	if err != nil {
		return 0, err
	}
	if user.AuthVersion <= 0 {
		return 0, openSourceBountyDeveloperAccessRequired()
	}
	return user.AuthVersion, nil
}

// openSourceBountyPrivateViewerId converts optional authentication into a
// private viewer only after a fresh L1 check. L0 and stale identities retain
// the same public board/detail view as anonymous callers.
func openSourceBountyPrivateViewerId(userId int) int {
	if RequireOpenSourceBountyDeveloperAccess(userId) != nil {
		return 0
	}
	return userId
}
