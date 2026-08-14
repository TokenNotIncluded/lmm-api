/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.
*/

package model

import (
	"errors"
	"regexp"
	"strings"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"gorm.io/gorm"
)

const (
	DiscountCodeStatusEnabled  = 1
	DiscountCodeStatusDisabled = 2
)

var discountCodePattern = regexp.MustCompile(`^[A-Z0-9][A-Z0-9_-]{2,63}$`)

var (
	ErrDiscountCodeNotFound = errors.New("discount code was not found")
	ErrDiscountCodeInactive = errors.New("discount code is inactive")
	ErrDiscountCodeExpired  = errors.New("discount code has expired")
	ErrDiscountCodeMinimum  = errors.New("discount code minimum amount was not met")
)

// DiscountCode is an administrator-managed percentage discount.  The code is
// intentionally separate from Redemption: redemption codes grant quota while
// discount codes only reduce the settlement amount of a purchase.
type DiscountCode struct {
	Id              int            `json:"id"`
	Code            string         `json:"code" gorm:"type:varchar(64);uniqueIndex"`
	Name            string         `json:"name" gorm:"type:varchar(120);index"`
	DiscountPercent int            `json:"discount_percent"`
	MinAmount       int64          `json:"min_amount" gorm:"not null;default:0"`
	Status          int            `json:"status" gorm:"not null;default:1;index"`
	UsedCount       int64          `json:"used_count" gorm:"not null;default:0"`
	CreatedBy       int            `json:"created_by" gorm:"index"`
	CreatedTime     int64          `json:"created_time" gorm:"not null;index"`
	UpdatedTime     int64          `json:"updated_time" gorm:"not null"`
	StartsTime      int64          `json:"starts_time" gorm:"not null;default:0"`
	ExpiredTime     int64          `json:"expired_time" gorm:"not null;default:0"`
	DeletedAt       gorm.DeletedAt `json:"-" gorm:"index"`
}

func NormalizeDiscountCode(value string) string {
	return strings.ToUpper(strings.TrimSpace(value))
}

func ValidateDiscountCodeDefinition(code string, percent int, minAmount, startsTime, expiredTime int64) error {
	if !discountCodePattern.MatchString(NormalizeDiscountCode(code)) {
		return errors.New("discount code must be 3-64 characters using A-Z, 0-9, _ or -")
	}
	if percent <= 0 || percent >= 100 {
		return errors.New("discount percent must be between 1 and 99")
	}
	if minAmount < 0 {
		return errors.New("minimum amount cannot be negative")
	}
	if startsTime < 0 || expiredTime < 0 || (startsTime > 0 && expiredTime > 0 && expiredTime <= startsTime) {
		return errors.New("invalid discount code validity window")
	}
	return nil
}

func GetAllDiscountCodes(startIdx, num int) (codes []*DiscountCode, total int64, err error) {
	query := DB.Model(&DiscountCode{})
	if err = query.Count(&total).Error; err != nil {
		return nil, 0, err
	}
	err = query.Order("id desc").Limit(num).Offset(startIdx).Find(&codes).Error
	return codes, total, err
}

func SearchDiscountCodes(keyword, status string, startIdx, num int) (codes []*DiscountCode, total int64, err error) {
	query := DB.Model(&DiscountCode{})
	if keyword = NormalizeDiscountCode(keyword); keyword != "" {
		query = query.Where("code LIKE ? OR name LIKE ?", keyword+"%", "%"+strings.TrimSpace(keyword)+"%")
	}
	if status != "" {
		query = query.Where("status = ?", status)
	}
	if err = query.Count(&total).Error; err != nil {
		return nil, 0, err
	}
	err = query.Order("id desc").Limit(num).Offset(startIdx).Find(&codes).Error
	return codes, total, err
}

func GetDiscountCodeById(id int) (*DiscountCode, error) {
	if id <= 0 {
		return nil, errors.New("invalid discount code id")
	}
	var code DiscountCode
	if err := DB.First(&code, "id = ?", id).Error; err != nil {
		return nil, err
	}
	return &code, nil
}

func GetDiscountCodeByValue(value string) (*DiscountCode, error) {
	code := NormalizeDiscountCode(value)
	if code == "" {
		return nil, ErrDiscountCodeNotFound
	}
	var row DiscountCode
	if err := DB.Where("code = ?", code).First(&row).Error; err != nil {
		if errors.Is(err, gorm.ErrRecordNotFound) {
			return nil, ErrDiscountCodeNotFound
		}
		return nil, err
	}
	return &row, nil
}

// ValidateDiscountCode checks only the current purchase eligibility.  It does
// not reserve usage; usage is counted atomically when the payment settles.
func ValidateDiscountCode(value string, amount int64, now int64) (*DiscountCode, error) {
	row, err := GetDiscountCodeByValue(value)
	if err != nil {
		return nil, err
	}
	if row.Status != DiscountCodeStatusEnabled {
		return nil, ErrDiscountCodeInactive
	}
	if row.StartsTime > 0 && row.StartsTime > now {
		return nil, ErrDiscountCodeInactive
	}
	if row.ExpiredTime > 0 && row.ExpiredTime < now {
		return nil, ErrDiscountCodeExpired
	}
	if amount < row.MinAmount {
		return nil, ErrDiscountCodeMinimum
	}
	return row, nil
}

func (code *DiscountCode) Insert() error {
	code.Code = NormalizeDiscountCode(code.Code)
	code.CreatedTime = common.GetTimestamp()
	code.UpdatedTime = code.CreatedTime
	return DB.Create(code).Error
}

func (code *DiscountCode) Update() error {
	code.Code = NormalizeDiscountCode(code.Code)
	code.UpdatedTime = common.GetTimestamp()
	return DB.Model(&DiscountCode{}).Where("id = ?", code.Id).Updates(map[string]interface{}{
		"code":             code.Code,
		"name":             code.Name,
		"discount_percent": code.DiscountPercent,
		"min_amount":       code.MinAmount,
		"status":           code.Status,
		"starts_time":      code.StartsTime,
		"expired_time":     code.ExpiredTime,
		"updated_time":     code.UpdatedTime,
	}).Error
}

func DeleteDiscountCodeById(id int) error {
	if id <= 0 {
		return errors.New("invalid discount code id")
	}
	return DB.Delete(&DiscountCode{}, id).Error
}
