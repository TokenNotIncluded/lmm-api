package model

import "gorm.io/gorm"

// lockAssistantOwner serializes private assistant writes with account
// deletion. A scoped lookup deliberately rejects soft-deleted owners.
func lockAssistantOwner(tx *gorm.DB, userID int) error {
	if tx == nil || userID <= 0 {
		return gorm.ErrInvalidData
	}
	var owner User
	return lockForUpdate(tx).Select("id").Where("id = ?", userID).First(&owner).Error
}

// deleteUserAssistantData removes private assistant state inside the caller's
// user-deletion transaction. Aggregate review and preset statistics are kept
// because they contain no user or conversation identifier.
func deleteUserAssistantData(tx *gorm.DB, userID int) error {
	if tx == nil || userID <= 0 {
		return gorm.ErrInvalidData
	}
	conversations := tx.Model(&AssistantConversation{}).Select("id").Where("user_id = ?", userID)
	incidents := tx.Model(&AssistantSecurityIncident{}).Select("id").
		Where("user_id = ? OR conversation_id IN (?)", userID, conversations)
	requests := tx.Model(&DeveloperAccessRequest{}).Select("id").Where("user_id = ?", userID)

	deletes := []struct {
		model any
		where string
		args  []any
	}{
		{&UnifiedTodoRead{}, "user_id = ?", []any{userID}},
		{&UnifiedTodoRead{}, "category = ? AND item_id IN (?)", []any{UnifiedTodoCategorySecurityIncident, incidents}},
		{&UnifiedTodoRead{}, "category = ? AND item_id IN (?)", []any{UnifiedTodoCategoryDeveloperAccess, requests}},
		{&PromptConversationRef{}, "conversation_id IN (?)", []any{conversations}},
		{&PromptConversionRef{}, "request_id IN (?)", []any{requests}},
		{&AssistantSecureCard{}, "owner_user_id = ? OR conversation_id IN (?)", []any{userID, conversations}},
		{&AssistantHistoryMessage{}, "conversation_id IN (?)", []any{conversations}},
		{&AssistantSecurityIncident{}, "user_id = ? OR conversation_id IN (?)", []any{userID, conversations}},
		{&AssistantConversation{}, "user_id = ?", []any{userID}},
		{&AssistantLead{}, "user_id = ?", []any{userID}},
		{&AssistantMemory{}, "user_id = ?", []any{userID}},
		{&AssistantUserProfile{}, "user_id = ?", []any{userID}},
		{&AssistantNewUserGift{}, "user_id = ?", []any{userID}},
		{&AdvancedSecurityEvent{}, "user_id = ?", []any{userID}},
		{&DeveloperAccessRequest{}, "user_id = ?", []any{userID}},
		{&AccountActionRequest{}, "target_user_id = ? OR requested_by_user_id = ?", []any{userID, userID}},
		{&L1OnboardingTodo{}, "user_id = ?", []any{userID}},
	}
	for _, deletion := range deletes {
		if err := tx.Unscoped().Where(deletion.where, deletion.args...).Delete(deletion.model).Error; err != nil {
			return err
		}
	}
	// A deleted administrator must not remain as the apparent resolver of
	// another user's support request.
	for _, record := range []any{&AssistantLead{}, &DeveloperAccessRequest{}, &AccountActionRequest{}} {
		if err := tx.Model(record).Where("admin_user_id = ?", userID).Update("admin_user_id", 0).Error; err != nil {
			return err
		}
	}
	for _, record := range []any{&AssistantMemory{}, &AssistantUserProfile{}} {
		if err := tx.Model(record).Where("updated_by = ?", userID).Update("updated_by", 0).Error; err != nil {
			return err
		}
	}
	return nil
}
