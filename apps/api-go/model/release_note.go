package model

import (
	"errors"
	"regexp"
	"strings"
	"unicode/utf8"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"gorm.io/gorm"
)

const (
	maxReleaseNoteVersionRunes = 128
	maxReleaseNoteContentRunes = 20000
)

var (
	ErrReleaseNoteVersionRequired = errors.New("release version is required")
	ErrReleaseNoteVersionTooLong  = errors.New("release version must be at most 128 characters")
	ErrReleaseNoteVersionInvalid  = errors.New("release version contains unsupported characters")
	ErrReleaseNoteContentRequired = errors.New("release changelog is required")
	ErrReleaseNoteContentTooLong  = errors.New("release changelog must be at most 20000 characters")
	ErrReleaseNoteNotFound        = errors.New("release note not found")

	releaseNoteVersionPattern = regexp.MustCompile(`^[0-9A-Za-z][0-9A-Za-z._+\-]*$`)
)

// ReleaseNote is one immutable changelog publication. Publishing the same
// version again creates a new revision so users can be notified of corrections.
type ReleaseNote struct {
	Id          int    `json:"id" gorm:"primaryKey"`
	Version     string `json:"version" gorm:"type:varchar(128);not null;uniqueIndex:idx_release_note_version_revision"`
	Revision    int    `json:"revision" gorm:"not null;uniqueIndex:idx_release_note_version_revision"`
	Content     string `json:"content" gorm:"type:text;not null"`
	PublishedAt int64  `json:"published_at" gorm:"not null;index"`
	PublishedBy int    `json:"published_by" gorm:"not null;index"`
}

func (ReleaseNote) TableName() string { return "release_notes" }

// ReleaseNoteRead makes acknowledgement durable across browser sessions and
// devices. The composite unique index also makes repeated acknowledgements safe.
type ReleaseNoteRead struct {
	Id            int   `json:"id" gorm:"primaryKey"`
	ReleaseNoteId int   `json:"release_note_id" gorm:"not null;uniqueIndex:idx_release_note_read_user_note"`
	UserId        int   `json:"user_id" gorm:"not null;uniqueIndex:idx_release_note_read_user_note"`
	ReadAt        int64 `json:"read_at" gorm:"not null"`
}

func (ReleaseNoteRead) TableName() string { return "release_note_reads" }

func normalizeReleaseNote(version string, content string) (string, string, error) {
	version = strings.TrimSpace(version)
	content = strings.TrimSpace(content)
	if version == "" {
		return "", "", ErrReleaseNoteVersionRequired
	}
	if utf8.RuneCountInString(version) > maxReleaseNoteVersionRunes {
		return "", "", ErrReleaseNoteVersionTooLong
	}
	if !releaseNoteVersionPattern.MatchString(version) {
		return "", "", ErrReleaseNoteVersionInvalid
	}
	if content == "" {
		return "", "", ErrReleaseNoteContentRequired
	}
	if utf8.RuneCountInString(content) > maxReleaseNoteContentRunes {
		return "", "", ErrReleaseNoteContentTooLong
	}
	return version, content, nil
}

func PublishReleaseNote(adminUserID int, version string, content string) (*ReleaseNote, error) {
	if adminUserID <= 0 {
		return nil, gorm.ErrInvalidData
	}
	version, content, err := normalizeReleaseNote(version, content)
	if err != nil {
		return nil, err
	}

	note := ReleaseNote{}
	err = DB.Transaction(func(tx *gorm.DB) error {
		latest := ReleaseNote{}
		revision := 1
		findErr := lockForUpdate(tx).
			Where("version = ?", version).
			Order("revision DESC").
			First(&latest).Error
		if findErr == nil {
			revision = latest.Revision + 1
		} else if !errors.Is(findErr, gorm.ErrRecordNotFound) {
			return findErr
		}

		note = ReleaseNote{
			Version:     version,
			Revision:    revision,
			Content:     content,
			PublishedAt: common.GetTimestamp(),
			PublishedBy: adminUserID,
		}
		return tx.Create(&note).Error
	})
	if err != nil {
		return nil, err
	}
	return &note, nil
}

// GetLatestUnreadReleaseNote returns only the newest publication. Users never
// receive a backlog of historical releases when the feature is first enabled.
func GetLatestUnreadReleaseNote(userID int, sessionCreatedAt int64) (*ReleaseNote, error) {
	if userID <= 0 {
		return nil, gorm.ErrInvalidData
	}
	var note ReleaseNote
	err := DB.Order("published_at DESC, id DESC").First(&note).Error
	if errors.Is(err, gorm.ErrRecordNotFound) {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	// A publication made after this browser session started waits for the next
	// login. This prevents a mid-session deployment from interrupting users.
	if sessionCreatedAt > 0 && note.PublishedAt > sessionCreatedAt {
		return nil, nil
	}

	var readCount int64
	if err := DB.Model(&ReleaseNoteRead{}).
		Where("release_note_id = ? AND user_id = ?", note.Id, userID).
		Count(&readCount).Error; err != nil {
		return nil, err
	}
	if readCount > 0 {
		return nil, nil
	}
	return &note, nil
}

func MarkReleaseNoteRead(userID int, releaseNoteID int) error {
	if userID <= 0 || releaseNoteID <= 0 {
		return gorm.ErrInvalidData
	}
	var noteCount int64
	if err := DB.Model(&ReleaseNote{}).Where("id = ?", releaseNoteID).Count(&noteCount).Error; err != nil {
		return err
	}
	if noteCount == 0 {
		return ErrReleaseNoteNotFound
	}
	read := ReleaseNoteRead{
		ReleaseNoteId: releaseNoteID,
		UserId:        userID,
		ReadAt:        common.GetTimestamp(),
	}
	return DB.Where("release_note_id = ? AND user_id = ?", releaseNoteID, userID).
		FirstOrCreate(&read).Error
}

func ListReleaseNotes(limit int) ([]ReleaseNote, error) {
	if limit <= 0 || limit > 100 {
		limit = 50
	}
	notes := make([]ReleaseNote, 0)
	err := DB.Order("published_at DESC, id DESC").Limit(limit).Find(&notes).Error
	return notes, err
}
