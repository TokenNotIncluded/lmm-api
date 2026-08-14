package model

import (
	"encoding/json"
	"errors"
	"sort"
	"strings"
	"unicode"
	"unicode/utf8"

	"github.com/QuantumNous/new-api/common"
	"gorm.io/gorm"
)

const (
	AssistantMemorySourceAssistant = "assistant"
	AssistantMemorySourceAdmin     = "administrator"
	AssistantMemoryMaxPerUser      = 64
	AssistantMemoryMaxTitleRunes   = 80
	AssistantMemoryMaxContentRunes = 800
	AssistantMemoryMaxTags         = 8
	AssistantMemoryRecallMax       = 4
)

var (
	ErrAssistantMemoryInvalid = errors.New("assistant memory is invalid")
	ErrAssistantMemoryLimit   = errors.New("assistant memory limit reached")
	ErrAssistantMemoryMissing = errors.New("assistant memory not found")
)

// AssistantMemory is a user-scoped skill. The owner ID is always selected by
// server-side authentication; it is never accepted from an assistant tool.
type AssistantMemory struct {
	Id        int64  `json:"id" gorm:"primaryKey"`
	UserId    int    `json:"-" gorm:"not null;uniqueIndex:idx_assistant_memory_owner_title,priority:1;index"`
	Title     string `json:"title" gorm:"type:varchar(160);not null;uniqueIndex:idx_assistant_memory_owner_title,priority:2"`
	Content   string `json:"content" gorm:"type:text;not null"`
	TagsJSON  string `json:"-" gorm:"type:text;not null;default:'[]'"`
	Source    string `json:"source" gorm:"type:varchar(24);not null"`
	Enabled   bool   `json:"enabled" gorm:"not null;default:true;index"`
	UpdatedBy int    `json:"-" gorm:"not null;default:0"`
	CreatedAt int64  `json:"created_at" gorm:"not null"`
	UpdatedAt int64  `json:"updated_at" gorm:"not null;index"`
}

func (AssistantMemory) TableName() string { return "assistant_memories" }

type AssistantMemoryView struct {
	Id        int64    `json:"id"`
	Title     string   `json:"title"`
	Content   string   `json:"content"`
	Tags      []string `json:"tags"`
	Source    string   `json:"source"`
	Enabled   bool     `json:"enabled"`
	CreatedAt int64    `json:"created_at"`
	UpdatedAt int64    `json:"updated_at"`
}

type MemoryInput struct {
	ID      int64
	Title   string
	Content string
	Tags    []string
	Source  string
	Enabled bool
}

func normalizeMemoryText(value string, limit int) (string, error) {
	value = strings.Map(func(r rune) rune {
		if unicode.IsControl(r) || unicode.In(r, unicode.Cf) {
			return -1
		}
		return r
	}, strings.TrimSpace(value))
	value = strings.Join(strings.Fields(value), " ")
	value = RedactAssistantHistoryContent(value)
	if value == "" || utf8.RuneCountInString(value) > limit {
		return "", ErrAssistantMemoryInvalid
	}
	return value, nil
}

func normalizeMemoryTags(tags []string) ([]string, error) {
	if len(tags) > AssistantMemoryMaxTags {
		return nil, ErrAssistantMemoryInvalid
	}
	result := make([]string, 0, len(tags))
	seen := make(map[string]struct{}, len(tags))
	for _, tag := range tags {
		normalized, err := normalizeMemoryText(tag, AssistantUserProfileMaxTagRunes)
		if err != nil {
			return nil, err
		}
		normalized = strings.ToLower(normalized)
		if _, exists := seen[normalized]; exists {
			continue
		}
		seen[normalized] = struct{}{}
		result = append(result, normalized)
	}
	return result, nil
}

func (memory AssistantMemory) Tags() []string {
	var tags []string
	if json.Unmarshal([]byte(memory.TagsJSON), &tags) != nil {
		return []string{}
	}
	return tags
}

func (memory AssistantMemory) View() AssistantMemoryView {
	return AssistantMemoryView{
		Id: memory.Id, Title: memory.Title, Content: memory.Content,
		Tags: memory.Tags(), Source: memory.Source, Enabled: memory.Enabled,
		CreatedAt: memory.CreatedAt, UpdatedAt: memory.UpdatedAt,
	}
}

func SaveMemory(ownerUserID, updatedBy int, input MemoryInput) (*AssistantMemory, error) {
	if ownerUserID <= 0 || updatedBy <= 0 || input.ID < 0 {
		return nil, gorm.ErrInvalidData
	}
	if input.Source != AssistantMemorySourceAssistant && input.Source != AssistantMemorySourceAdmin {
		return nil, ErrAssistantMemoryInvalid
	}
	var err error
	input.Title, err = normalizeMemoryText(input.Title, AssistantMemoryMaxTitleRunes)
	if err != nil {
		return nil, err
	}
	input.Content, err = normalizeMemoryText(input.Content, AssistantMemoryMaxContentRunes)
	if err != nil {
		return nil, err
	}
	input.Tags, err = normalizeMemoryTags(input.Tags)
	if err != nil {
		return nil, err
	}
	tagsJSON, _ := json.Marshal(input.Tags)
	now := common.GetTimestamp()
	var saved AssistantMemory
	err = DB.Transaction(func(tx *gorm.DB) error {
		if err := lockAssistantOwner(tx, ownerUserID); err != nil {
			return err
		}
		if input.ID > 0 {
			if err := lockForUpdate(tx).Where("id = ? AND user_id = ?", input.ID, ownerUserID).First(&saved).Error; err != nil {
				if errors.Is(err, gorm.ErrRecordNotFound) {
					return ErrAssistantMemoryMissing
				}
				return err
			}
			saved.Title, saved.Content, saved.TagsJSON = input.Title, input.Content, string(tagsJSON)
			saved.Source, saved.Enabled, saved.UpdatedBy, saved.UpdatedAt = input.Source, input.Enabled, updatedBy, now
			return tx.Save(&saved).Error
		}
		// A repeated assistant observation updates the same owner/title memory.
		if err := lockForUpdate(tx).Where("user_id = ? AND title = ?", ownerUserID, input.Title).First(&saved).Error; err == nil {
			saved.Content, saved.TagsJSON = input.Content, string(tagsJSON)
			saved.Source, saved.Enabled, saved.UpdatedBy, saved.UpdatedAt = input.Source, input.Enabled, updatedBy, now
			return tx.Save(&saved).Error
		} else if !errors.Is(err, gorm.ErrRecordNotFound) {
			return err
		}
		var count int64
		if err := tx.Model(&AssistantMemory{}).Where("user_id = ?", ownerUserID).Count(&count).Error; err != nil {
			return err
		}
		if count >= AssistantMemoryMaxPerUser {
			return ErrAssistantMemoryLimit
		}
		saved = AssistantMemory{
			UserId: ownerUserID, Title: input.Title, Content: input.Content, TagsJSON: string(tagsJSON),
			Source: input.Source, Enabled: input.Enabled, UpdatedBy: updatedBy, CreatedAt: now, UpdatedAt: now,
		}
		return tx.Create(&saved).Error
	})
	if err != nil {
		return nil, err
	}
	return &saved, nil
}

func ListMemories(ownerUserID int, includeDisabled bool) ([]AssistantMemoryView, error) {
	if ownerUserID <= 0 {
		return nil, gorm.ErrInvalidData
	}
	query := DB.Where("user_id = ?", ownerUserID)
	if !includeDisabled {
		query = query.Where("enabled = ?", true)
	}
	var rows []AssistantMemory
	if err := query.Order("updated_at DESC, id DESC").Limit(AssistantMemoryMaxPerUser).Find(&rows).Error; err != nil {
		return nil, err
	}
	views := make([]AssistantMemoryView, 0, len(rows))
	for _, row := range rows {
		views = append(views, row.View())
	}
	return views, nil
}

func RecallMemories(ownerUserID int, query string, limit int) ([]AssistantMemoryView, error) {
	if ownerUserID <= 0 {
		return nil, gorm.ErrInvalidData
	}
	if limit <= 0 || limit > AssistantMemoryRecallMax {
		limit = AssistantMemoryRecallMax
	}
	var rows []AssistantMemory
	if err := DB.Where("user_id = ? AND enabled = ?", ownerUserID, true).
		Order("updated_at DESC, id DESC").Limit(AssistantMemoryMaxPerUser).Find(&rows).Error; err != nil {
		return nil, err
	}
	terms := strings.Fields(strings.ToLower(strings.TrimSpace(query)))
	if len(terms) > 12 {
		terms = terms[:12]
	}
	type scoredMemory struct {
		row   AssistantMemory
		score int
	}
	scored := make([]scoredMemory, 0, len(rows))
	for _, row := range rows {
		title := strings.ToLower(row.Title)
		content := strings.ToLower(row.Content)
		tags := strings.ToLower(row.TagsJSON)
		score := 0
		for _, term := range terms {
			if strings.Contains(title, term) {
				score += 4
			}
			if strings.Contains(tags, term) {
				score += 2
			}
			if strings.Contains(content, term) {
				score++
			}
		}
		if len(terms) == 0 || score > 0 {
			scored = append(scored, scoredMemory{row: row, score: score})
		}
	}
	sort.SliceStable(scored, func(i, j int) bool {
		if scored[i].score != scored[j].score {
			return scored[i].score > scored[j].score
		}
		return scored[i].row.UpdatedAt > scored[j].row.UpdatedAt
	})
	if len(scored) > limit {
		scored = scored[:limit]
	}
	views := make([]AssistantMemoryView, 0, len(scored))
	for _, item := range scored {
		views = append(views, item.row.View())
	}
	return views, nil
}

func DeleteMemory(ownerUserID int, memoryID int64) error {
	if ownerUserID <= 0 || memoryID <= 0 {
		return gorm.ErrInvalidData
	}
	return DB.Transaction(func(tx *gorm.DB) error {
		if err := lockAssistantOwner(tx, ownerUserID); err != nil {
			return err
		}
		result := tx.Where("id = ? AND user_id = ?", memoryID, ownerUserID).Delete(&AssistantMemory{})
		if result.Error != nil {
			return result.Error
		}
		if result.RowsAffected == 0 {
			return ErrAssistantMemoryMissing
		}
		return nil
	})
}
