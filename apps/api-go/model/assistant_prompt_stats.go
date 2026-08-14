package model

import (
	"github.com/QuantumNous/new-api/common"
	"gorm.io/gorm"
	"gorm.io/gorm/clause"
)

type presetEvent string

const (
	presetClick          presetEvent = "click"
	presetConversation   presetEvent = "conversation"
	presetRecommendation presetEvent = "recommendation"
	presetApproval       presetEvent = "approval"
)

func countPresetTx(db *gorm.DB, attribution PromptPresetRef, event presetEvent) error {
	if attribution.PresetId == "" || attribution.Version == "" {
		return ErrPromptPresetNotFound
	}
	now := common.GetTimestamp()
	const bucketSeconds int64 = 60 * 60
	bucketStart := now - now%bucketSeconds
	column := map[presetEvent]string{
		presetClick:          "click_count",
		presetConversation:   "conversation_count",
		presetRecommendation: "recommendation_count",
		presetApproval:       "approval_count",
	}[event]
	if column == "" {
		return gorm.ErrInvalidData
	}
	increment := gorm.Expr(column+" + ?", 1)
	if common.UsingMainDatabase(common.DatabaseTypePostgreSQL) {
		increment = gorm.Expr(`"assistant_pre_conversation_preset_stats".`+column+" + ?", 1)
	}
	updates := map[string]any{
		column:       increment,
		"updated_at": now,
	}
	row := PromptPresetStat{
		PresetId: attribution.PresetId, BucketStart: bucketStart, Generation: attribution.Generation,
		Version: attribution.Version, UpdatedAt: now,
	}
	switch event {
	case presetClick:
		row.ClickCount = 1
	case presetConversation:
		row.ConversationCount = 1
	case presetRecommendation:
		row.RecommendationCount = 1
	case presetApproval:
		row.ApprovalCount = 1
	}
	return db.Clauses(clause.OnConflict{
		Columns:   []clause.Column{{Name: "preset_id"}, {Name: "bucket_start"}, {Name: "generation"}, {Name: "version"}},
		DoUpdates: clause.Assignments(updates),
	}).Create(&row).Error
}

func countPreset(attribution PromptPresetRef, event presetEvent) error {
	return countPresetTx(DB, attribution, event)
}

func CountPresetClick(presetId string) error {
	attribution, _, err := findPromptPreset(presetId)
	if err != nil {
		return err
	}
	return countPreset(*attribution, presetClick)
}

func CountPresetConversation(attribution PromptPresetRef, conversationId int64) error {
	if conversationId <= 0 {
		return gorm.ErrInvalidData
	}
	return DB.Transaction(func(tx *gorm.DB) error {
		if err := countPresetTx(tx, attribution, presetConversation); err != nil {
			return err
		}
		row := PromptConversationRef{
			ConversationId: conversationId, PresetId: attribution.PresetId, Generation: attribution.Generation,
			Version: attribution.Version, UpdatedAt: common.GetTimestamp(),
		}
		return tx.Clauses(clause.OnConflict{
			Columns:   []clause.Column{{Name: "conversation_id"}},
			DoUpdates: clause.AssignmentColumns([]string{"preset_id", "generation", "version", "updated_at"}),
		}).Create(&row).Error
	})
}

func ConversationPreset(conversationId int64) (*PromptPresetRef, error) {
	if conversationId <= 0 {
		return nil, gorm.ErrInvalidData
	}
	var row PromptConversationRef
	if err := DB.Where("conversation_id = ?", conversationId).Take(&row).Error; err != nil {
		return nil, err
	}
	return &PromptPresetRef{
		PresetId: row.PresetId, Generation: row.Generation, Version: row.Version,
	}, nil
}

func CountPresetRecommendation(attribution PromptPresetRef, requestId int) error {
	if requestId <= 0 {
		return gorm.ErrInvalidData
	}
	return DB.Transaction(func(tx *gorm.DB) error {
		if err := countPresetTx(tx, attribution, presetRecommendation); err != nil {
			return err
		}
		row := PromptConversionRef{
			RequestId: requestId, PresetId: attribution.PresetId, Generation: attribution.Generation,
			Version: attribution.Version, UpdatedAt: common.GetTimestamp(),
		}
		return tx.Clauses(clause.OnConflict{
			Columns:   []clause.Column{{Name: "request_id"}},
			DoUpdates: clause.AssignmentColumns([]string{"preset_id", "generation", "version", "updated_at"}),
		}).Create(&row).Error
	})
}

func CountPresetApproval(requestId int) error {
	if requestId <= 0 {
		return gorm.ErrInvalidData
	}
	return DB.Transaction(func(tx *gorm.DB) error {
		var row PromptConversionRef
		if err := tx.Where("request_id = ?", requestId).Take(&row).Error; err != nil {
			return err
		}
		attribution := PromptPresetRef{
			PresetId: row.PresetId, Generation: row.Generation, Version: row.Version,
		}
		if err := countPresetTx(tx, attribution, presetApproval); err != nil {
			return err
		}
		return tx.Delete(&row).Error
	})
}
