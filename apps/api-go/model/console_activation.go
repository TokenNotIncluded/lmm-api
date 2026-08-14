package model

import (
	"errors"
	"fmt"
	"time"

	"github.com/LIghtJUNction/api.lmm.best/common"

	"gorm.io/gorm"
)

const existingUsersL1BackfillOptionKey = "migration.existing_users_l1.v1"

// ConsoleActivationNeedsLegacyBackfill must be sampled before AutoMigrate.
// A missing column means every row already present belongs to a pre-rollout
// account and must retain full console access after the column is added.
func ConsoleActivationNeedsLegacyBackfill() bool {
	return !DB.Migrator().HasColumn(&User{}, "ConsoleActivatedAt")
}

// InitializeLegacyConsoleActivations is intentionally conditional. Running an
// unconditional zero-value backfill on every boot would activate accounts
// registered after rollout before they create their first credential.
func InitializeLegacyConsoleActivations(backfill bool) error {
	if !backfill {
		return nil
	}
	return DB.Model(&User{}).
		Where("console_activated_at = ?", 0).
		Update("console_activated_at", time.Now().Unix()).Error
}

// InitializeExistingUsersL1Backfill grants the one-time compatibility floor
// requested for accounts that already existed when the L1 policy shipped.
// The Option marker is written in the same transaction so later boots never
// activate users who register after this migration has completed.
func InitializeExistingUsersL1Backfill() error {
	var marker Option
	err := DB.Where("key = ?", existingUsersL1BackfillOptionKey).First(&marker).Error
	if err == nil {
		return nil
	}
	if !errors.Is(err, gorm.ErrRecordNotFound) {
		return err
	}

	return DB.Transaction(func(tx *gorm.DB) error {
		var existing Option
		checkErr := lockForUpdate(tx).Where("key = ?", existingUsersL1BackfillOptionKey).First(&existing).Error
		if checkErr == nil {
			return nil
		}
		if !errors.Is(checkErr, gorm.ErrRecordNotFound) {
			return checkErr
		}
		now := time.Now().Unix()
		if err := tx.Model(&User{}).
			Where("console_activated_at = ?", 0).
			Update("console_activated_at", now).Error; err != nil {
			return fmt.Errorf("activate existing users at L1: %w", err)
		}
		// Trust levels are automatic. Clear any legacy administrator override
		// while establishing the one-time compatibility floor.
		if err := tx.Model(&User{}).Where("1 = 1").Update("trust_level_override", nil).Error; err != nil {
			return fmt.Errorf("clear legacy trust overrides: %w", err)
		}
		return tx.Create(&Option{Key: existingUsersL1BackfillOptionKey, Value: fmt.Sprintf("%d", now)}).Error
	})
}

// InsertTokenAndActivateConsole commits the first credential and permanent
// console activation together. Deleting every credential later does not
// revoke the activation timestamp.
func InsertTokenAndActivateConsole(token *Token) error {
	if token == nil || token.UserId <= 0 {
		return gorm.ErrInvalidData
	}
	now := time.Now().Unix()
	err := DB.Transaction(func(tx *gorm.DB) error {
		if err := tx.Create(token).Error; err != nil {
			return err
		}
		return tx.Model(&User{}).
			Where("id = ? AND console_activated_at = ?", token.UserId, 0).
			Update("console_activated_at", now).Error
	})
	if err != nil {
		return err
	}
	if err := invalidateUserCache(token.UserId); err != nil {
		common.SysLog("failed to invalidate user cache after console activation: " + err.Error())
	}
	return nil
}
