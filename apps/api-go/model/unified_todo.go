package model

import (
	"errors"
	"sort"
	"strconv"
	"strings"

	"github.com/QuantumNous/new-api/common"
	"gorm.io/gorm"
	"gorm.io/gorm/clause"
)

const (
	UnifiedTodoCategoryAll             = "all"
	UnifiedTodoCategoryBounty          = "open_source_bounty"
	UnifiedTodoCategoryDeveloperAccess = "developer_access"
	UnifiedTodoCategoryAccountAction   = "account_action"

	maxUnifiedTodoPage     = 100
	maxUnifiedTodoPageSize = 50
	defaultUnifiedTodoSize = 20
)

var (
	ErrUnifiedTodoCategory = errors.New("待办分类无效")
	ErrUnifiedTodoReadBody = errors.New("待办已读请求无效")
)

// UnifiedTodoRead stores per-viewer acknowledgement for sources that do not
// already have a recipient_read_at column. It intentionally contains only an
// opaque source identifier and never copies source payloads or credentials.
type UnifiedTodoRead struct {
	Id       int    `json:"id" gorm:"primaryKey"`
	UserId   int    `json:"user_id" gorm:"not null;uniqueIndex:idx_unified_todo_read,priority:1;index"`
	Category string `json:"category" gorm:"type:varchar(40);not null;uniqueIndex:idx_unified_todo_read,priority:2"`
	ItemId   int    `json:"item_id" gorm:"not null;uniqueIndex:idx_unified_todo_read,priority:3"`
	ReadAt   int64  `json:"read_at" gorm:"not null"`
}

func (UnifiedTodoRead) TableName() string { return "unified_todo_reads" }

type UnifiedTodoItem struct {
	Id        string         `json:"id"`
	SourceId  int            `json:"source_id"`
	Category  string         `json:"category"`
	Type      string         `json:"type"`
	Title     string         `json:"title"`
	Summary   string         `json:"summary"`
	Read      bool           `json:"read"`
	CreatedAt int64          `json:"created_at"`
	UpdatedAt int64          `json:"updated_at"`
	Details   map[string]any `json:"details,omitempty"`
}

type UnifiedTodoCategorySummary struct {
	Key    string `json:"key"`
	Total  int64  `json:"total"`
	Unread int64  `json:"unread"`
}

type UnifiedTodoPage struct {
	Items            []UnifiedTodoItem            `json:"items"`
	Page             int                          `json:"page"`
	PageSize         int                          `json:"page_size"`
	Total            int64                        `json:"total"`
	Category         string                       `json:"category"`
	UnreadCount      int64                        `json:"unread_count"`
	TotalUnreadCount int64                        `json:"total_unread_count"`
	UnreadByCategory map[string]int64             `json:"unread_by_category"`
	Categories       []UnifiedTodoCategorySummary `json:"categories"`
}

type unifiedTodoCandidate struct {
	Item UnifiedTodoItem
}

var unifiedTodoCategories = []string{
	UnifiedTodoCategoryBounty,
	UnifiedTodoCategoryDeveloperAccess,
	UnifiedTodoCategoryAccountAction,
}

func normalizeUnifiedTodoCategory(category string) (string, error) {
	category = strings.TrimSpace(category)
	if category == "" {
		return UnifiedTodoCategoryAll, nil
	}
	if category == UnifiedTodoCategoryAll {
		return category, nil
	}
	for _, known := range unifiedTodoCategories {
		if category == known {
			return category, nil
		}
	}
	return "", ErrUnifiedTodoCategory
}

func normalizeUnifiedTodoPage(page, pageSize int) (int, int) {
	if page < 1 {
		page = 1
	}
	if page > maxUnifiedTodoPage {
		page = maxUnifiedTodoPage
	}
	if pageSize < 1 {
		pageSize = defaultUnifiedTodoSize
	}
	if pageSize > maxUnifiedTodoPageSize {
		pageSize = maxUnifiedTodoPageSize
	}
	return page, pageSize
}

func unifiedTodoItemID(category string, sourceID int) string {
	return category + ":" + strconv.Itoa(sourceID)
}

func unifiedTodoSelectedCategories(category string) []string {
	if category == UnifiedTodoCategoryAll {
		return append([]string(nil), unifiedTodoCategories...)
	}
	return []string{category}
}

func unifiedTodoNotificationTitle(kind string) string {
	switch kind {
	case OpenSourceBountyLedgerTipTransfer:
		return "open_source_bounty.tip_received"
	case OpenSourceBountyLedgerRewardTransfer:
		return "open_source_bounty.reward_received"
	case OpenSourceBountyLedgerDisputeRewardTransfer:
		return "open_source_bounty.dispute_reward_received"
	default:
		return "open_source_bounty.notification"
	}
}

func unifiedTodoBountyCandidates(userID, limit int) ([]unifiedTodoCandidate, error) {
	notifications := make([]OpenSourceBountyNotification, 0)
	if err := openSourceBountyNotificationQuery().
		Where("notification.kind IN ? AND notification.counterparty_user_id = ?", openSourceBountyNotificationKinds(), userID).
		Order("notification.created_at DESC, notification.id DESC").Limit(limit).Scan(&notifications).Error; err != nil {
		return nil, err
	}

	items := make([]unifiedTodoCandidate, 0, len(notifications))
	for _, notification := range notifications {
		items = append(items, unifiedTodoCandidate{Item: UnifiedTodoItem{
			Id:        unifiedTodoItemID(UnifiedTodoCategoryBounty, notification.Id),
			SourceId:  notification.Id,
			Category:  UnifiedTodoCategoryBounty,
			Type:      notification.Kind,
			Title:     unifiedTodoNotificationTitle(notification.Kind),
			Summary:   RedactAssistantHistoryContent(notification.ProjectTitle),
			Read:      notification.RecipientReadAt > 0,
			CreatedAt: notification.CreatedAt,
			UpdatedAt: notification.CreatedAt,
			Details: map[string]any{
				"project_id":      notification.ProjectId,
				"challenge_id":    notification.ChallengeId,
				"sender_username": notification.SenderUsername,
				"project_title":   RedactAssistantHistoryContent(notification.ProjectTitle),
				"quota":           notification.Quota,
				"note":            RedactAssistantHistoryContent(notification.Note),
				"thanked":         notification.ThankedAt > 0,
			},
		}})
	}
	return items, nil
}

func unifiedDeveloperAccessQuery(userID int, isAdmin bool) *gorm.DB {
	query := DB.Table("developer_access_requests AS request").
		Select("request.*, users.username, users.email").
		Joins("JOIN users ON users.id = request.user_id AND users.deleted_at IS NULL")
	if !isAdmin {
		query = query.Where("request.user_id = ?", userID)
	}
	return query
}

func unifiedAccountActionQuery(userID int, isAdmin bool) *gorm.DB {
	query := DB.Table("account_action_requests AS request").
		Select(`request.*, target.username AS target_username, target.email AS target_email,
			requester.username AS requested_by_username, requester.email AS requested_by_email`).
		Joins("JOIN users AS target ON target.id = request.target_user_id AND target.deleted_at IS NULL").
		Joins("LEFT JOIN users AS requester ON requester.id = request.requested_by_user_id AND requester.deleted_at IS NULL")
	if !isAdmin {
		query = query.Where("(request.target_user_id = ? OR request.requested_by_user_id = ?)", userID, userID)
	}
	return query
}

func unifiedDeveloperAccessCandidates(userID, limit int, isAdmin bool) ([]unifiedTodoCandidate, error) {
	rows := make([]DeveloperAccessRequestView, 0)
	if err := unifiedDeveloperAccessQuery(userID, isAdmin).
		Order("CASE WHEN request.reviewed_at > 0 THEN request.reviewed_at ELSE request.created_at END DESC, request.id DESC").
		Limit(limit).Find(&rows).Error; err != nil {
		return nil, err
	}

	items := make([]unifiedTodoCandidate, 0, len(rows))
	for _, row := range rows {
		updatedAt := row.CreatedAt
		if row.ReviewedAt > 0 {
			updatedAt = row.ReviewedAt
		}
		details := map[string]any{
			"request_id":        row.Id,
			"status":            row.Status,
			"source":            row.Source,
			"reason":            RedactAssistantHistoryContent(row.Reason),
			"ai_recommendation": RedactAssistantHistoryContent(row.AIRecommendation),
			"admin_note":        RedactAssistantHistoryContent(row.AdminNote),
		}
		if isAdmin {
			details["user_id"] = row.UserId
			details["username"] = row.Username
		}
		items = append(items, unifiedTodoCandidate{Item: UnifiedTodoItem{
			Id:        unifiedTodoItemID(UnifiedTodoCategoryDeveloperAccess, row.Id),
			SourceId:  row.Id,
			Category:  UnifiedTodoCategoryDeveloperAccess,
			Type:      row.Status,
			Title:     "developer_access.request",
			Summary:   "developer access request: " + row.Status,
			CreatedAt: row.CreatedAt,
			UpdatedAt: updatedAt,
			Details:   details,
		}})
	}
	return items, nil
}

func unifiedAccountActionCandidates(userID, limit int, isAdmin bool) ([]unifiedTodoCandidate, error) {
	rows := make([]AccountActionRequestView, 0)
	if err := unifiedAccountActionQuery(userID, isAdmin).
		Order("CASE WHEN request.reviewed_at > 0 THEN request.reviewed_at ELSE request.created_at END DESC, request.id DESC").
		Limit(limit).Find(&rows).Error; err != nil {
		return nil, err
	}

	items := make([]unifiedTodoCandidate, 0, len(rows))
	for _, row := range rows {
		updatedAt := row.CreatedAt
		if row.ReviewedAt > 0 {
			updatedAt = row.ReviewedAt
		}
		details := map[string]any{
			"request_id": row.Id,
			"kind":       row.Kind,
			"status":     row.Status,
			"reason":     RedactAssistantHistoryContent(row.Reason),
			"admin_note": RedactAssistantHistoryContent(row.AdminNote),
		}
		if isAdmin {
			details["target_user_id"] = row.TargetUserId
			details["target_username"] = row.TargetUsername
			details["requested_by_user_id"] = row.RequestedByUserId
			details["requested_by_username"] = row.RequestedByUsername
		}
		items = append(items, unifiedTodoCandidate{Item: UnifiedTodoItem{
			Id:        unifiedTodoItemID(UnifiedTodoCategoryAccountAction, row.Id),
			SourceId:  row.Id,
			Category:  UnifiedTodoCategoryAccountAction,
			Type:      row.Kind + "." + row.Status,
			Title:     "account_action.request",
			Summary:   row.Kind + " account request: " + row.Status,
			CreatedAt: row.CreatedAt,
			UpdatedAt: updatedAt,
			Details:   details,
		}})
	}
	return items, nil
}

func unifiedTodoCount(query *gorm.DB) (int64, error) {
	var count int64
	if err := query.Count(&count).Error; err != nil {
		return 0, err
	}
	return count, nil
}

func unifiedBountyCount(userID int, unreadOnly bool) (int64, error) {
	query := openSourceBountyNotificationQuery().
		Where("notification.kind IN ? AND notification.counterparty_user_id = ?", openSourceBountyNotificationKinds(), userID)
	if unreadOnly {
		query = query.Where("notification.recipient_read_at = 0")
	}
	return unifiedTodoCount(query)
}

func unifiedDeveloperAccessCount(userID int, isAdmin bool, unreadOnly bool) (int64, error) {
	query := unifiedDeveloperAccessQuery(userID, isAdmin)
	if unreadOnly {
		query = query.Where(`NOT EXISTS (
			SELECT 1 FROM unified_todo_reads AS read_marker
			WHERE read_marker.user_id = ? AND read_marker.category = ? AND read_marker.item_id = request.id
		)`, userID, UnifiedTodoCategoryDeveloperAccess)
	}
	return unifiedTodoCount(query)
}

func unifiedAccountActionCount(userID int, isAdmin bool, unreadOnly bool) (int64, error) {
	query := unifiedAccountActionQuery(userID, isAdmin)
	if unreadOnly {
		query = query.Where(`NOT EXISTS (
			SELECT 1 FROM unified_todo_reads AS read_marker
			WHERE read_marker.user_id = ? AND read_marker.category = ? AND read_marker.item_id = request.id
		)`, userID, UnifiedTodoCategoryAccountAction)
	}
	return unifiedTodoCount(query)
}

func loadUnifiedTodoReadMap(userID int, category string, ids []int) (map[int]bool, error) {
	result := make(map[int]bool, len(ids))
	if len(ids) == 0 {
		return result, nil
	}
	var rows []UnifiedTodoRead
	if err := DB.Where("user_id = ? AND category = ? AND item_id IN ?", userID, category, ids).Find(&rows).Error; err != nil {
		return nil, err
	}
	for _, row := range rows {
		result[row.ItemId] = true
	}
	return result, nil
}

func applyUnifiedTodoReadMap(candidates []unifiedTodoCandidate, userID int) error {
	byCategory := map[string][]int{}
	for _, candidate := range candidates {
		if candidate.Item.Category == UnifiedTodoCategoryBounty {
			continue
		}
		byCategory[candidate.Item.Category] = append(byCategory[candidate.Item.Category], candidate.Item.SourceId)
	}
	readMaps := make(map[string]map[int]bool, len(byCategory))
	for category, ids := range byCategory {
		readMap, err := loadUnifiedTodoReadMap(userID, category, ids)
		if err != nil {
			return err
		}
		readMaps[category] = readMap
	}
	for index := range candidates {
		if readMaps[candidates[index].Item.Category] != nil {
			candidates[index].Item.Read = readMaps[candidates[index].Item.Category][candidates[index].Item.SourceId]
		}
	}
	return nil
}

// GetUnifiedTodoCenter returns only rows visible to userID. Ordinary users get
// their own bounty notifications and review results; administrators additionally
// get the complete developer-access and account-action queues.
func GetUnifiedTodoCenter(userID, role int, category string, page, pageSize int) (*UnifiedTodoPage, error) {
	if userID <= 0 {
		return nil, gorm.ErrInvalidData
	}
	category, err := normalizeUnifiedTodoCategory(category)
	if err != nil {
		return nil, err
	}
	page, pageSize = normalizeUnifiedTodoPage(page, pageSize)
	isAdmin := role >= common.RoleAdminUser

	counts := make(map[string]UnifiedTodoCategorySummary, len(unifiedTodoCategories))
	for _, knownCategory := range unifiedTodoCategories {
		var total, unread int64
		switch knownCategory {
		case UnifiedTodoCategoryBounty:
			total, err = unifiedBountyCount(userID, false)
			if err == nil {
				unread, err = unifiedBountyCount(userID, true)
			}
		case UnifiedTodoCategoryDeveloperAccess:
			total, err = unifiedDeveloperAccessCount(userID, isAdmin, false)
			if err == nil {
				unread, err = unifiedDeveloperAccessCount(userID, isAdmin, true)
			}
		case UnifiedTodoCategoryAccountAction:
			total, err = unifiedAccountActionCount(userID, isAdmin, false)
			if err == nil {
				unread, err = unifiedAccountActionCount(userID, isAdmin, true)
			}
		}
		if err != nil {
			return nil, err
		}
		counts[knownCategory] = UnifiedTodoCategorySummary{Key: knownCategory, Total: total, Unread: unread}
	}

	allUnread := int64(0)
	for _, knownCategory := range unifiedTodoCategories {
		allUnread += counts[knownCategory].Unread
	}
	selectedTotal := int64(0)
	selectedUnread := int64(0)
	for _, selectedCategory := range unifiedTodoSelectedCategories(category) {
		selectedTotal += counts[selectedCategory].Total
		selectedUnread += counts[selectedCategory].Unread
	}

	limit := page * pageSize
	candidates := make([]unifiedTodoCandidate, 0)
	for _, selectedCategory := range unifiedTodoSelectedCategories(category) {
		var categoryCandidates []unifiedTodoCandidate
		switch selectedCategory {
		case UnifiedTodoCategoryBounty:
			categoryCandidates, err = unifiedTodoBountyCandidates(userID, limit)
		case UnifiedTodoCategoryDeveloperAccess:
			categoryCandidates, err = unifiedDeveloperAccessCandidates(userID, limit, isAdmin)
		case UnifiedTodoCategoryAccountAction:
			categoryCandidates, err = unifiedAccountActionCandidates(userID, limit, isAdmin)
		}
		if err != nil {
			return nil, err
		}
		candidates = append(candidates, categoryCandidates...)
	}
	if err := applyUnifiedTodoReadMap(candidates, userID); err != nil {
		return nil, err
	}
	sort.SliceStable(candidates, func(i, j int) bool {
		left, right := candidates[i].Item, candidates[j].Item
		if left.UpdatedAt != right.UpdatedAt {
			return left.UpdatedAt > right.UpdatedAt
		}
		if left.Category != right.Category {
			return left.Category < right.Category
		}
		return left.SourceId > right.SourceId
	})

	start := (page - 1) * pageSize
	items := make([]UnifiedTodoItem, 0, pageSize)
	if start < len(candidates) {
		end := start + pageSize
		if end > len(candidates) {
			end = len(candidates)
		}
		for _, candidate := range candidates[start:end] {
			items = append(items, candidate.Item)
		}
	}

	categorySummaries := make([]UnifiedTodoCategorySummary, 0, len(unifiedTodoCategories))
	unreadByCategory := make(map[string]int64, len(unifiedTodoCategories))
	for _, knownCategory := range unifiedTodoCategories {
		categorySummaries = append(categorySummaries, counts[knownCategory])
		unreadByCategory[knownCategory] = counts[knownCategory].Unread
	}
	return &UnifiedTodoPage{
		Items:            items,
		Page:             page,
		PageSize:         pageSize,
		Total:            selectedTotal,
		Category:         category,
		UnreadCount:      selectedUnread,
		TotalUnreadCount: allUnread,
		UnreadByCategory: unreadByCategory,
		Categories:       categorySummaries,
	}, nil
}

func visibleUnifiedTodoIDs(userID, role int, category string, ids []int, all bool) ([]int, error) {
	isAdmin := role >= common.RoleAdminUser
	var visible []int
	switch category {
	case UnifiedTodoCategoryDeveloperAccess:
		query := DB.Model(&DeveloperAccessRequest{}).Select("id")
		if !isAdmin {
			query = query.Where("user_id = ?", userID)
		}
		if !all {
			query = query.Where("id IN ?", ids)
		}
		if err := query.Pluck("id", &visible).Error; err != nil {
			return nil, err
		}
	case UnifiedTodoCategoryAccountAction:
		query := DB.Model(&AccountActionRequest{}).Select("id")
		if !isAdmin {
			query = query.Where("(target_user_id = ? OR requested_by_user_id = ?)", userID, userID)
		}
		if !all {
			query = query.Where("id IN ?", ids)
		}
		if err := query.Pluck("id", &visible).Error; err != nil {
			return nil, err
		}
	default:
		return nil, ErrUnifiedTodoCategory
	}
	return visible, nil
}

func markUnifiedGenericTodosRead(userID, role int, category string, ids []int, all bool) (int, error) {
	visible, err := visibleUnifiedTodoIDs(userID, role, category, ids, all)
	if err != nil {
		return 0, err
	}
	if len(visible) == 0 {
		return 0, nil
	}
	rows := make([]UnifiedTodoRead, 0, len(visible))
	now := common.GetTimestamp()
	for _, itemID := range visible {
		rows = append(rows, UnifiedTodoRead{UserId: userID, Category: category, ItemId: itemID, ReadAt: now})
	}
	result := DB.Clauses(clause.OnConflict{DoNothing: true}).Create(&rows)
	return int(result.RowsAffected), result.Error
}

func markUnifiedBountyTodosRead(userID int, ids []int, all bool) (int, error) {
	query := DB.Model(&OpenSourceBountyLedger{}).
		Where("kind IN ? AND counterparty_user_id = ? AND recipient_read_at = 0", openSourceBountyNotificationKinds(), userID)
	if !all {
		query = query.Where("id IN ?", ids)
	}
	result := query.Update("recipient_read_at", common.GetTimestamp())
	return int(result.RowsAffected), result.Error
}

// MarkUnifiedTodoReads acknowledges either selected rows or all visible rows
// in one category. For category=all, all=true is required so an accidental
// cross-category ID cannot mark an unrelated request as read.
func MarkUnifiedTodoReads(userID, role int, category string, ids []int, all bool) (int, error) {
	if userID <= 0 {
		return 0, gorm.ErrInvalidData
	}
	category, err := normalizeUnifiedTodoCategory(category)
	if err != nil {
		return 0, err
	}
	if category == UnifiedTodoCategoryAll {
		if !all || len(ids) > 0 {
			return 0, ErrUnifiedTodoReadBody
		}
		total := 0
		for _, knownCategory := range unifiedTodoCategories {
			marked, markErr := MarkUnifiedTodoReads(userID, role, knownCategory, nil, true)
			if markErr != nil {
				return total, markErr
			}
			total += marked
		}
		return total, nil
	}
	if !all && len(ids) == 0 {
		return 0, ErrUnifiedTodoReadBody
	}
	if !all {
		seen := make(map[int]struct{}, len(ids))
		normalized := make([]int, 0, len(ids))
		for _, id := range ids {
			if id <= 0 {
				return 0, ErrUnifiedTodoReadBody
			}
			if _, exists := seen[id]; !exists {
				seen[id] = struct{}{}
				normalized = append(normalized, id)
			}
		}
		ids = normalized
	}
	if category == UnifiedTodoCategoryBounty {
		return markUnifiedBountyTodosRead(userID, ids, all)
	}
	return markUnifiedGenericTodosRead(userID, role, category, ids, all)
}
