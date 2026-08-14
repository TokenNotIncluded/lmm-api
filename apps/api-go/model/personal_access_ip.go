package model

import (
	"errors"
	"net/netip"
	"strings"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"gorm.io/gorm"
)

const PersonalAccessIPMinTrustLevel = TrustLevelMinUser + 1

var (
	ErrPersonalAccessIPNotEligible = errors.New("personal IP allowlist requires trust level L1 or higher")
	ErrInvalidPersonalAccessIP     = errors.New("IP address must be a public, globally routable address")
)

// PersonalAccessIP stores the single source address an eligible account may
// use to bypass the production mainland-China ingress block. The user id is
// the primary key, so each account can hold at most one address. Multiple
// accounts may intentionally share a public egress address.
type PersonalAccessIP struct {
	UserId    int    `json:"user_id" gorm:"primaryKey;column:user_id"`
	IP        string `json:"ip" gorm:"type:varchar(45);not null;index"`
	CreatedAt int64  `json:"created_at" gorm:"autoCreateTime;column:created_at"`
	UpdatedAt int64  `json:"updated_at" gorm:"autoUpdateTime;column:updated_at"`
}

func (PersonalAccessIP) TableName() string {
	return "personal_access_ips"
}

func normalizePersonalAccessIP(raw string) (string, error) {
	value := strings.TrimSpace(raw)
	if value == "" {
		return "", ErrInvalidPersonalAccessIP
	}
	addr, err := netip.ParseAddr(value)
	if err != nil || !addr.IsValid() || addr.Zone() != "" {
		return "", ErrInvalidPersonalAccessIP
	}
	addr = addr.Unmap()
	if !addr.IsGlobalUnicast() || addr.IsPrivate() || addr.IsLoopback() ||
		addr.IsLinkLocalUnicast() || addr.IsLinkLocalMulticast() ||
		addr.IsMulticast() || addr.IsUnspecified() || isReservedPersonalAccessIP(addr) {
		return "", ErrInvalidPersonalAccessIP
	}
	return addr.String(), nil
}

func isReservedPersonalAccessIP(addr netip.Addr) bool {
	reserved := []netip.Prefix{
		netip.MustParsePrefix("100.64.0.0/10"),   // shared address space / CGNAT
		netip.MustParsePrefix("192.0.0.0/24"),    // IETF protocol assignments
		netip.MustParsePrefix("192.0.2.0/24"),    // TEST-NET-1
		netip.MustParsePrefix("198.18.0.0/15"),   // benchmarking
		netip.MustParsePrefix("198.51.100.0/24"), // TEST-NET-2
		netip.MustParsePrefix("203.0.113.0/24"),  // TEST-NET-3
		netip.MustParsePrefix("2001:db8::/32"),   // IPv6 documentation
	}
	for _, prefix := range reserved {
		if prefix.Contains(addr) {
			return true
		}
	}
	return false
}

func NormalizePersonalAccessIP(raw string) (string, error) {
	return normalizePersonalAccessIP(raw)
}

func GetPersonalAccessIP(userID int) (*PersonalAccessIP, error) {
	if userID <= 0 {
		return nil, gorm.ErrInvalidData
	}
	if DB == nil {
		return nil, gorm.ErrInvalidDB
	}
	var record PersonalAccessIP
	err := DB.Where("user_id = ?", userID).First(&record).Error
	if errors.Is(err, gorm.ErrRecordNotFound) {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	return &record, nil
}

func DeletePersonalAccessIP(userID int) error {
	if userID <= 0 || DB == nil {
		if DB == nil {
			return gorm.ErrInvalidDB
		}
		return gorm.ErrInvalidData
	}
	return DB.Where("user_id = ?", userID).Delete(&PersonalAccessIP{}).Error
}

func userCanManagePersonalAccessIP(user *User) (bool, error) {
	if user == nil {
		return false, gorm.ErrInvalidData
	}
	trust, err := GetFreshTrustLevelInfoForUser(user)
	if err != nil {
		return false, err
	}
	return trust.Level >= PersonalAccessIPMinTrustLevel, nil
}

// SetPersonalAccessIP validates eligibility and atomically replaces the
// account's existing address.
func SetPersonalAccessIP(user *User, rawIP string) (*PersonalAccessIP, error) {
	if user == nil || user.Id <= 0 {
		return nil, gorm.ErrInvalidData
	}
	allowed, err := userCanManagePersonalAccessIP(user)
	if err != nil {
		return nil, err
	}
	if !allowed {
		return nil, ErrPersonalAccessIPNotEligible
	}
	ip, err := normalizePersonalAccessIP(rawIP)
	if err != nil {
		return nil, err
	}
	if DB == nil {
		return nil, gorm.ErrInvalidDB
	}

	record := &PersonalAccessIP{UserId: user.Id, IP: ip}
	err = DB.Transaction(func(tx *gorm.DB) error {
		var current PersonalAccessIP
		findErr := tx.Where("user_id = ?", user.Id).First(&current).Error
		if findErr == nil {
			current.IP = ip
			return tx.Save(&current).Error
		}
		if !errors.Is(findErr, gorm.ErrRecordNotFound) {
			return findErr
		}
		return tx.Create(record).Error
	})
	if err != nil {
		return nil, err
	}
	return record, nil
}

// IsPersonalAccessIPAllowedForUser is intentionally fail-closed. It is called
// by the loopback-only Nginx auth_request endpoint, so a database failure
// cannot accidentally turn the mainland-China gate into an allow rule. The
// user id is part of the lookup: a valid account may only use its own address,
// never another account's allowlist entry.
func IsPersonalAccessIPAllowedForUser(userID int, rawIP string) (bool, error) {
	if userID <= 0 {
		return false, gorm.ErrInvalidData
	}
	ip, err := normalizePersonalAccessIP(rawIP)
	if err != nil {
		return false, nil
	}
	if DB == nil {
		return false, gorm.ErrInvalidDB
	}
	var record PersonalAccessIP
	if err := DB.Where("user_id = ? AND ip = ?", userID, ip).First(&record).Error; err != nil {
		if errors.Is(err, gorm.ErrRecordNotFound) {
			return false, nil
		}
		return false, err
	}
	user, err := GetUserById(record.UserId, false)
	if err != nil {
		return false, err
	}
	if user.Status != common.UserStatusEnabled {
		return false, nil
	}
	allowed, err := userCanManagePersonalAccessIP(user)
	if err != nil {
		return false, err
	}
	return allowed, nil
}
