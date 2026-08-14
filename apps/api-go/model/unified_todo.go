package model

import (
	"errors"
	"strconv"
	"strings"

	"github.com/QuantumNous/new-api/common"
	"gorm.io/gorm"
	"gorm.io/gorm/clause"
)

const (
	UnifiedTodoCategoryAll              = "all"
	UnifiedTodoCategoryBounty           = "open_source_bounty"
	UnifiedTodoCategoryBountyReview     = "open_source_bounty_review"
	UnifiedTodoCategoryDeveloperAccess  = "developer_access"
	UnifiedTodoCategoryAccountAction    = "account_action"
	UnifiedTodoCategorySecurityIncident = "security_incident"

	maxUnifiedTodoPage     = 100
	maxUnifiedTodoPageSize = 50
	defaultUnifiedTodoSize = 20
	maxUnifiedTodoReadIDs  = 100
	unifiedTodoReadBatch   = 200
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
	Category string `json:"category" gorm:"type:varchar(40);not null;uniqueIndex:idx_unified_todo_read,priority:2;index:idx_unified_todo_source,priority:1"`
	ItemId   int    `json:"item_id" gorm:"not null;uniqueIndex:idx_unified_todo_read,priority:3;index:idx_unified_todo_source,priority:2"`
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

// todoRef is the small, fixed-width row used for cross-source pagination.
// Full todo payloads are loaded only for the selected page, so deep pages do
// not retain page*size objects from every source in Go memory.
type todoRef struct {
	SourceID int    `gorm:"column:source_id"`
	Category string `gorm:"column:category"`
	Updated  int64  `gorm:"column:updated_at"`
}

var unifiedTodoCategories = []string{
	UnifiedTodoCategorySecurityIncident,
	UnifiedTodoCategoryBountyReview,
	UnifiedTodoCategoryBounty,
	UnifiedTodoCategoryDeveloperAccess,
	UnifiedTodoCategoryAccountAction,
}

type unifiedAssistantSecurityIncidentView struct {
	AssistantSecurityIncident
	Username string `gorm:"column:username"`
}

func unifiedSecurityIncidentQuery(viewerRole int) *gorm.DB {
	query := DB.Table("assistant_security_incidents AS incident").
		Select("incident.*, users.username").
		Joins("JOIN users ON users.id = incident.user_id AND users.deleted_at IS NULL")
	if viewerRole < common.RoleAdminUser {
		return query.Where("1 = 0")
	}
	return query.Where("users.role < ? AND incident.status = ?", viewerRole, AssistantSecurityIncidentStatusOpen)
}

func unifiedSecurityIncidentCandidates(viewerRole int, ids []int) ([]unifiedTodoCandidate, error) {
	if len(ids) == 0 {
		return []unifiedTodoCandidate{}, nil
	}
	rows := make([]unifiedAssistantSecurityIncidentView, 0)
	if err := unifiedSecurityIncidentQuery(viewerRole).
		Where("incident.id IN ?", ids).Scan(&rows).Error; err != nil {
		return nil, err
	}

	items := make([]unifiedTodoCandidate, 0, len(rows))
	for _, row := range rows {
		items = append(items, unifiedTodoCandidate{Item: UnifiedTodoItem{
			Id:        unifiedTodoItemID(UnifiedTodoCategorySecurityIncident, row.Id),
			SourceId:  row.Id,
			Category:  UnifiedTodoCategorySecurityIncident,
			Type:      row.Category,
			Title:     "assistant.security_incident",
			Summary:   "assistant conversation ended by safety policy",
			CreatedAt: row.CreatedAt,
			UpdatedAt: row.UpdatedAt,
			Details: map[string]any{
				"user_id":         row.UserId,
				"username":        row.Username,
				"conversation_id": row.ConversationId,
				"status":          row.Status,
			},
		}})
	}
	return items, nil
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

func todoRefs(userID, role int, category string, offset, limit int) ([]todoRef, error) {
	selected := make(map[string]bool, len(unifiedTodoCategories))
	for _, key := range unifiedTodoSelectedCategories(category) {
		selected[key] = true
	}
	isAdmin := role >= common.RoleAdminUser
	parts := make([]string, 0, len(selected))
	args := make([]any, 0, 24)
	add := func(query string, values ...any) {
		parts = append(parts, query)
		args = append(args, values...)
	}

	if selected[UnifiedTodoCategorySecurityIncident] && isAdmin {
		add(`SELECT incident.id AS source_id, ? AS category, incident.updated_at AS updated_at
			FROM assistant_security_incidents AS incident
			JOIN users ON users.id = incident.user_id AND users.deleted_at IS NULL
			WHERE users.role < ? AND incident.status = ?`,
			UnifiedTodoCategorySecurityIncident, role, AssistantSecurityIncidentStatusOpen)
	}
	if selected[UnifiedTodoCategoryBountyReview] {
		add(`SELECT challenge.id AS source_id, ? AS category, challenge.updated_at AS updated_at
			FROM open_source_bounty_challenges AS challenge
			JOIN open_source_bounty_projects AS project ON project.id = challenge.project_id
			WHERE project.owner_user_id = ? AND challenge.status = ?`,
			UnifiedTodoCategoryBountyReview, userID, OpenSourceBountyChallengeSubmitted)
	}
	if selected[UnifiedTodoCategoryBounty] {
		add(`SELECT notification.id AS source_id, ? AS category, notification.created_at AS updated_at
			FROM open_source_bounty_ledgers AS notification
			JOIN users AS sender ON sender.id = notification.user_id AND sender.deleted_at IS NULL
			JOIN open_source_bounty_projects AS project ON project.id = notification.project_id
			WHERE notification.kind IN ? AND notification.counterparty_user_id = ?`,
			UnifiedTodoCategoryBounty, openSourceBountyNotificationKinds(), userID)
	}
	if selected[UnifiedTodoCategoryDeveloperAccess] {
		query := `SELECT request.id AS source_id, ? AS category, request.created_at AS updated_at
			FROM developer_access_requests AS request
			JOIN users ON users.id = request.user_id AND users.deleted_at IS NULL
			WHERE request.status = ? AND request.source <> ?`
		values := []any{UnifiedTodoCategoryDeveloperAccess, DeveloperAccessRequestPending, DeveloperAccessRequestSourceOld}
		if !isAdmin {
			query += " AND request.user_id = ?"
			values = append(values, userID)
		}
		add(query, values...)
	}
	if selected[UnifiedTodoCategoryAccountAction] {
		query := `SELECT request.id AS source_id, ? AS category,
			CASE WHEN request.reviewed_at > 0 THEN request.reviewed_at ELSE request.created_at END AS updated_at
			FROM account_action_requests AS request
			JOIN users AS target ON target.id = request.target_user_id AND target.deleted_at IS NULL
			WHERE request.status = ?`
		values := []any{UnifiedTodoCategoryAccountAction, AccountActionStatusPending}
		if !isAdmin {
			query += " AND (request.target_user_id = ? OR request.requested_by_user_id = ?)"
			values = append(values, userID, userID)
		}
		add(query, values...)
	}
	if len(parts) == 0 {
		return []todoRef{}, nil
	}

	query := "SELECT source_id, category, updated_at FROM (" + strings.Join(parts, " UNION ALL ") +
		") AS todo ORDER BY updated_at DESC, category ASC, source_id DESC LIMIT ? OFFSET ?"
	args = append(args, limit, offset)
	refs := make([]todoRef, 0, limit)
	if err := DB.Raw(query, args...).Scan(&refs).Error; err != nil {
		return nil, err
	}
	return refs, nil
}

func loadTodoCandidates(userID, role int, refs []todoRef) ([]unifiedTodoCandidate, error) {
	ids := make(map[string][]int, len(unifiedTodoCategories))
	for _, ref := range refs {
		ids[ref.Category] = append(ids[ref.Category], ref.SourceID)
	}
	isAdmin := role >= common.RoleAdminUser
	loaded := make([]unifiedTodoCandidate, 0, len(refs))
	for _, category := range unifiedTodoCategories {
		var items []unifiedTodoCandidate
		var err error
		switch category {
		case UnifiedTodoCategorySecurityIncident:
			items, err = unifiedSecurityIncidentCandidates(role, ids[category])
		case UnifiedTodoCategoryBountyReview:
			items, err = unifiedTodoBountyReviewCandidates(userID, ids[category])
		case UnifiedTodoCategoryBounty:
			items, err = unifiedTodoBountyCandidates(userID, ids[category])
		case UnifiedTodoCategoryDeveloperAccess:
			items, err = unifiedDeveloperAccessCandidates(userID, ids[category], isAdmin)
		case UnifiedTodoCategoryAccountAction:
			items, err = unifiedAccountActionCandidates(userID, ids[category], isAdmin)
		}
		if err != nil {
			return nil, err
		}
		loaded = append(loaded, items...)
	}

	byID := make(map[string]unifiedTodoCandidate, len(loaded))
	for _, item := range loaded {
		byID[item.Item.Id] = item
	}
	ordered := make([]unifiedTodoCandidate, 0, len(refs))
	for _, ref := range refs {
		id := unifiedTodoItemID(ref.Category, ref.SourceID)
		if item, ok := byID[id]; ok {
			ordered = append(ordered, item)
		}
	}
	return ordered, nil
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

func unifiedTodoBountyCandidates(userID int, ids []int) ([]unifiedTodoCandidate, error) {
	if len(ids) == 0 {
		return []unifiedTodoCandidate{}, nil
	}
	notifications := make([]OpenSourceBountyNotification, 0)
	if err := openSourceBountyNotificationQuery().
		Where("notification.kind IN ? AND notification.counterparty_user_id = ?", openSourceBountyNotificationKinds(), userID).
		Where("notification.id IN ?", ids).Scan(&notifications).Error; err != nil {
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

func unifiedTodoBountyReviewQuery(userID int) *gorm.DB {
	return openSourceBountyChallengeViewQuery().
		Where("p.owner_user_id = ? AND c.status = ?", userID, OpenSourceBountyChallengeSubmitted)
}

func unifiedTodoBountyReviewCandidates(userID int, ids []int) ([]unifiedTodoCandidate, error) {
	if len(ids) == 0 {
		return []unifiedTodoCandidate{}, nil
	}
	rows := make([]OpenSourceBountyChallengeView, 0)
	if err := unifiedTodoBountyReviewQuery(userID).
		Where("c.id IN ?", ids).Scan(&rows).Error; err != nil {
		return nil, err
	}

	items := make([]unifiedTodoCandidate, 0, len(rows))
	for _, row := range rows {
		items = append(items, unifiedTodoCandidate{Item: UnifiedTodoItem{
			Id:        unifiedTodoItemID(UnifiedTodoCategoryBountyReview, row.Id),
			SourceId:  row.Id,
			Category:  UnifiedTodoCategoryBountyReview,
			Type:      "challenge_submitted",
			Title:     "open_source_bounty.challenge_submitted",
			Summary:   RedactAssistantHistoryContent(row.ProjectTitle),
			CreatedAt: row.SubmittedAt,
			UpdatedAt: row.UpdatedAt,
			Details: map[string]any{
				"project_id":           row.ProjectId,
				"challenge_id":         row.Id,
				"participant_user_id":  row.ParticipantUserId,
				"participant_username": row.ParticipantUsername,
				"project_title":        RedactAssistantHistoryContent(row.ProjectTitle),
				"issue_url":            row.IssueUrl,
				"pull_request_url":     row.PullRequestUrl,
				"submission_note":      RedactAssistantHistoryContent(row.SubmissionNote),
				"status":               row.Status,
			},
		}})
	}
	return items, nil
}

func unifiedDeveloperAccessQuery(userID int, isAdmin bool) *gorm.DB {
	query := DB.Table("developer_access_requests AS request").
		Select("request.*, users.username, users.email").
		Joins("JOIN users ON users.id = request.user_id AND users.deleted_at IS NULL").
		Where("request.status = ? AND request.source <> ?", DeveloperAccessRequestPending, DeveloperAccessRequestSourceOld)
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
		Joins("LEFT JOIN users AS requester ON requester.id = request.requested_by_user_id AND requester.deleted_at IS NULL").
		Where("request.status = ?", AccountActionStatusPending)
	if !isAdmin {
		query = query.Where("(request.target_user_id = ? OR request.requested_by_user_id = ?)", userID, userID)
	}
	return query
}

func unifiedDeveloperAccessCandidates(userID int, ids []int, isAdmin bool) ([]unifiedTodoCandidate, error) {
	if len(ids) == 0 {
		return []unifiedTodoCandidate{}, nil
	}
	rows := make([]DeveloperAccessRequestView, 0)
	if err := unifiedDeveloperAccessQuery(userID, isAdmin).
		Where("request.id IN ?", ids).Find(&rows).Error; err != nil {
		return nil, err
	}

	items := make([]unifiedTodoCandidate, 0, len(rows))
	for _, row := range rows {
		summary := RedactAssistantHistoryContent(row.AIRecommendation)
		if strings.TrimSpace(summary) == "" {
			summary = RedactAssistantHistoryContent(row.Reason)
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
			details["email"] = row.Email
		}
		items = append(items, unifiedTodoCandidate{Item: UnifiedTodoItem{
			Id:        unifiedTodoItemID(UnifiedTodoCategoryDeveloperAccess, row.Id),
			SourceId:  row.Id,
			Category:  UnifiedTodoCategoryDeveloperAccess,
			Type:      row.Status,
			Title:     "developer_access.request",
			Summary:   summary,
			CreatedAt: row.CreatedAt,
			UpdatedAt: row.CreatedAt,
			Details:   details,
		}})
	}
	return items, nil
}

func unifiedAccountActionCandidates(userID int, ids []int, isAdmin bool) ([]unifiedTodoCandidate, error) {
	if len(ids) == 0 {
		return []unifiedTodoCandidate{}, nil
	}
	rows := make([]AccountActionRequestView, 0)
	if err := unifiedAccountActionQuery(userID, isAdmin).
		Where("request.id IN ?", ids).Find(&rows).Error; err != nil {
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

func unifiedBountyReviewCount(userID int, unreadOnly bool) (int64, error) {
	query := unifiedTodoBountyReviewQuery(userID)
	if unreadOnly {
		query = query.Where(`NOT EXISTS (
			SELECT 1 FROM unified_todo_reads AS read_marker
			WHERE read_marker.user_id = ? AND read_marker.category = ? AND read_marker.item_id = c.id
		)`, userID, UnifiedTodoCategoryBountyReview)
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

func unifiedSecurityIncidentCount(userID, role int, unreadOnly bool) (int64, error) {
	query := unifiedSecurityIncidentQuery(role)
	if unreadOnly {
		query = query.Where(`NOT EXISTS (
			SELECT 1 FROM unified_todo_reads AS read_marker
			WHERE read_marker.user_id = ? AND read_marker.category = ? AND read_marker.item_id = incident.id
		)`, userID, UnifiedTodoCategorySecurityIncident)
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
		case UnifiedTodoCategorySecurityIncident:
			total, err = unifiedSecurityIncidentCount(userID, role, false)
			if err == nil {
				unread, err = unifiedSecurityIncidentCount(userID, role, true)
			}
		case UnifiedTodoCategoryBountyReview:
			total, err = unifiedBountyReviewCount(userID, false)
			if err == nil {
				unread, err = unifiedBountyReviewCount(userID, true)
			}
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

	start := (page - 1) * pageSize
	refs, err := todoRefs(userID, role, category, start, pageSize)
	if err != nil {
		return nil, err
	}
	candidates, err := loadTodoCandidates(userID, role, refs)
	if err != nil {
		return nil, err
	}
	if err := applyUnifiedTodoReadMap(candidates, userID); err != nil {
		return nil, err
	}
	items := make([]UnifiedTodoItem, 0, len(candidates))
	for _, candidate := range candidates {
		items = append(items, candidate.Item)
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

func visibleTodoQuery(userID, role int, category string) (*gorm.DB, string, error) {
	isAdmin := role >= common.RoleAdminUser
	switch category {
	case UnifiedTodoCategorySecurityIncident:
		return unifiedSecurityIncidentQuery(role).Select("incident.id"), "incident.id", nil
	case UnifiedTodoCategoryBountyReview:
		query := DB.Table("open_source_bounty_challenges AS c").
			Select("c.id").
			Joins("JOIN open_source_bounty_projects AS p ON p.id = c.project_id").
			Where("p.owner_user_id = ? AND c.status = ?", userID, OpenSourceBountyChallengeSubmitted)
		return query, "c.id", nil
	case UnifiedTodoCategoryDeveloperAccess:
		query := DB.Model(&DeveloperAccessRequest{}).Select("id").
			Where("status = ? AND source <> ?", DeveloperAccessRequestPending, DeveloperAccessRequestSourceOld)
		if !isAdmin {
			query = query.Where("user_id = ?", userID)
		}
		return query, "id", nil
	case UnifiedTodoCategoryAccountAction:
		query := DB.Model(&AccountActionRequest{}).Select("id").
			Where("status = ?", AccountActionStatusPending)
		if !isAdmin {
			query = query.Where("(target_user_id = ? OR requested_by_user_id = ?)", userID, userID)
		}
		return query, "id", nil
	default:
		return nil, "", ErrUnifiedTodoCategory
	}
}

func markUnifiedGenericTodosRead(userID, role int, category string, ids []int, all bool) (int, error) {
	query, idColumn, err := visibleTodoQuery(userID, role, category)
	if err != nil {
		return 0, err
	}
	if !all {
		var visible []int
		if err := query.Where(idColumn+" IN ?", ids).Pluck(idColumn, &visible).Error; err != nil {
			return 0, err
		}
		return insertTodoReads(userID, category, visible)
	}

	total := 0
	cursor := 0
	for {
		visible := make([]int, 0, unifiedTodoReadBatch)
		page := query.Session(&gorm.Session{}).
			Where(idColumn+" > ?", cursor).
			Order(idColumn + " ASC").
			Limit(unifiedTodoReadBatch)
		if err := page.Pluck(idColumn, &visible).Error; err != nil {
			return total, err
		}
		if len(visible) == 0 {
			return total, nil
		}
		marked, err := insertTodoReads(userID, category, visible)
		if err != nil {
			return total, err
		}
		total += marked
		cursor = visible[len(visible)-1]
	}
}

func insertTodoReads(userID int, category string, ids []int) (int, error) {
	if len(ids) == 0 {
		return 0, nil
	}
	rows := make([]UnifiedTodoRead, len(ids))
	now := common.GetTimestamp()
	for index, itemID := range ids {
		rows[index] = UnifiedTodoRead{UserId: userID, Category: category, ItemId: itemID, ReadAt: now}
	}
	result := DB.Clauses(clause.OnConflict{DoNothing: true}).CreateInBatches(&rows, unifiedTodoReadBatch)
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
		if len(ids) > maxUnifiedTodoReadIDs {
			return 0, ErrUnifiedTodoReadBody
		}
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
