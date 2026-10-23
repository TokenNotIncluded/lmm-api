package assistant_services

import (
	"context"
	"database/sql"
	"errors"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/redaction"
	"strings"
	"unicode/utf8"
)

// SupportMessages never grants an administrator transcript access merely from
// their platform role. Only the customer and currently assigned administrator
// may read this request; every invocation rechecks the original Core session.
func (s *Service) SupportMessages(ctx context.Context, credential string, id, after int64) ([]SupportMessage, error) {
	p, err := s.principal(ctx, credential)
	if err != nil {
		return nil, err
	}
	if id <= 0 || after < 0 {
		return nil, access.ErrInvalid
	}
	tx, err := s.db.BeginTx(ctx, &sql.TxOptions{ReadOnly: true})
	if err != nil {
		return nil, access.ErrUnavailable
	}
	defer tx.Rollback()
	request, err := scanSupport(tx.QueryRowContext(ctx, `SELECT `+supportColumns+` FROM lmm_assistant.platform_support WHERE id=$1`, id))
	if errors.Is(err, sql.ErrNoRows) {
		return nil, access.ErrNotFound
	}
	if err != nil {
		return nil, access.ErrUnavailable
	}
	if request.UserID != p.UserID && !(p.Level >= 5 && request.AssignedAdminID != nil && *request.AssignedAdminID == p.UserID) {
		return nil, access.ErrForbidden
	}
	rows, err := tx.QueryContext(ctx, `SELECT id,support_id,author_id,author_kind,content,created_at FROM lmm_assistant.platform_support_messages WHERE support_id=$1 AND id>$2 ORDER BY id LIMIT 50`, id, after)
	if err != nil {
		return nil, access.ErrUnavailable
	}
	defer rows.Close()
	out := []SupportMessage{}
	for rows.Next() {
		var m SupportMessage
		if rows.Scan(&m.ID, &m.SupportID, &m.AuthorID, &m.Role, &m.Content, &m.CreatedAt) != nil {
			return nil, access.ErrUnavailable
		}
		m.Content = redaction.Content(m.Content)
		out = append(out, m)
	}
	if rows.Err() != nil {
		return nil, access.ErrUnavailable
	}
	return out, nil
}

// SendSupportMessage binds an idempotency key to exact normalized content. A
// conflicting replay is rejected; closed requests cannot receive late writes.
func (s *Service) SendSupportMessage(ctx context.Context, credential string, id int64, clientTurn, content string) (SupportMessage, error) {
	p, err := s.principal(ctx, credential)
	if err != nil {
		return SupportMessage{}, err
	}
	content = strings.TrimSpace(redaction.Content(content))
	if id <= 0 || !access.ID(clientTurn) || !utf8.ValidString(content) || utf8.RuneCountInString(content) < 1 || utf8.RuneCountInString(content) > 4000 {
		return SupportMessage{}, access.ErrInvalid
	}
	tx, err := s.db.BeginTx(ctx, nil)
	if err != nil {
		return SupportMessage{}, access.ErrUnavailable
	}
	defer tx.Rollback()
	request, err := scanSupport(tx.QueryRowContext(ctx, `SELECT `+supportColumns+` FROM lmm_assistant.platform_support WHERE id=$1 FOR UPDATE`, id))
	if errors.Is(err, sql.ErrNoRows) {
		return SupportMessage{}, access.ErrNotFound
	}
	if err != nil {
		return SupportMessage{}, access.ErrUnavailable
	}
	role := "user"
	if request.UserID != p.UserID {
		if p.Level < 5 || request.AssignedAdminID == nil || *request.AssignedAdminID != p.UserID {
			return SupportMessage{}, access.ErrForbidden
		}
		role = "human"
	}
	var m SupportMessage
	err = tx.QueryRowContext(ctx, `SELECT id,support_id,author_id,author_kind,content,created_at FROM lmm_assistant.platform_support_messages WHERE support_id=$1 AND author_id=$2 AND client_turn_id=$3`, id, p.UserID, clientTurn).Scan(&m.ID, &m.SupportID, &m.AuthorID, &m.Role, &m.Content, &m.CreatedAt)
	if err == nil {
		if m.Content != content || m.Role != role {
			return m, access.ErrConflict
		}
		return m, nil
	}
	if !errors.Is(err, sql.ErrNoRows) {
		return m, access.ErrUnavailable
	}
	if request.ClosedAt != nil {
		return m, access.ErrConflict
	}
	if role == "human" && request.Status != "accepted" {
		return m, access.ErrConflict
	}
	err = tx.QueryRowContext(ctx, `INSERT INTO lmm_assistant.platform_support_messages(support_id,author_id,author_kind,content,client_turn_id,created_at) VALUES($1,$2,$3,$4,$5,$6) RETURNING id,support_id,author_id,author_kind,content,created_at`, id, p.UserID, role, content, clientTurn, s.now().Unix()).Scan(&m.ID, &m.SupportID, &m.AuthorID, &m.Role, &m.Content, &m.CreatedAt)
	if err != nil {
		return m, access.ErrUnavailable
	}
	if tx.Commit() != nil {
		return m, access.ErrUnavailable
	}
	return m, nil
}

// HumanSupportActive is a fresh per-turn pause boundary for the assistant host.
// A pending or accepted immediate handoff pauses AI in this exact conversation;
// an appointment only pauses once a human has actually accepted it.
func (s *Service) HumanSupportActive(ctx context.Context, credential, coreConversation string) (bool, error) {
	p, err := s.principal(ctx, credential)
	if err != nil {
		return false, err
	}
	if coreConversation == "" || len(coreConversation) > 128 {
		return false, access.ErrInvalid
	}
	var active bool
	err = s.db.QueryRowContext(ctx, `SELECT EXISTS(SELECT FROM lmm_assistant.platform_support WHERE user_id=$1 AND core_conversation_id=$2 AND (status='accepted' OR (kind='handoff' AND status='pending')))`, p.UserID, coreConversation).Scan(&active)
	if err != nil {
		return false, access.ErrUnavailable
	}
	return active, nil
}
