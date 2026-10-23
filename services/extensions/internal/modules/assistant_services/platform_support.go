// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later
// Platform assistant support is separate from merchant/buyer store support.
package assistant_services

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"strings"
	"time"
	"unicode/utf8"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/redaction"
)

type SupportRequest struct {
	ID                 int64  `json:"id"`
	UserID             int64  `json:"user_id"`
	CoreConversationID string `json:"core_conversation_id"`
	Kind               string `json:"kind"`
	Status             string `json:"status"`
	Topic              string `json:"topic"`
	PreferredTime      string `json:"preferred_time"`
	ScheduledAt        int64  `json:"scheduled_at"`
	AssignedAdminID    *int64 `json:"assigned_admin_id"`
	AssignedAdminName  string `json:"assigned_admin_name"`
	CreatedAt          int64  `json:"created_at"`
	UpdatedAt          int64  `json:"updated_at"`
	AcceptedAt         *int64 `json:"accepted_at"`
	ClosedAt           *int64 `json:"closed_at"`
	Revision           int64  `json:"revision"`
}
type SupportMessage struct {
	ID        int64  `json:"id"`
	SupportID int64  `json:"support_id"`
	AuthorID  int64  `json:"author_id"`
	Role      string `json:"role"`
	Content   string `json:"content"`
	CreatedAt int64  `json:"created_at"`
}
type BookInput struct {
	Topic         string `json:"topic"`
	PreferredTime string `json:"preferred_time"`
	ScheduledAt   int64  `json:"scheduled_at"`
}
type handoffPayload struct {
	Message            string `json:"message"`
	CoreConversationID string `json:"core_conversation_id"`
}

const supportColumns = `id,user_id,core_conversation_id,kind,status,topic,preferred_time,scheduled_at,assigned_admin_id,assigned_admin_name,created_at,updated_at,accepted_at,closed_at,revision`

func scanSupport(row interface{ Scan(...any) error }) (SupportRequest, error) {
	var request SupportRequest
	err := row.Scan(&request.ID, &request.UserID, &request.CoreConversationID, &request.Kind, &request.Status, &request.Topic, &request.PreferredTime, &request.ScheduledAt, &request.AssignedAdminID, &request.AssignedAdminName, &request.CreatedAt, &request.UpdatedAt, &request.AcceptedAt, &request.ClosedAt, &request.Revision)
	if err == nil {
		request.Topic = redaction.Content(request.Topic)
		request.PreferredTime = redaction.Content(request.PreferredTime)
		request.AssignedAdminName = redaction.Content(request.AssignedAdminName)
	}
	return request, err
}
func (s *Service) eligibility(ctx context.Context, credential string) (Eligibility, error) {
	if access.Missing(s.dependencies.SupportEligibility) {
		return Eligibility{}, access.ErrUnavailable
	}
	proof, err := s.dependencies.SupportEligibility.SupportEligibility(ctx, credential)
	if err != nil {
		return proof, err
	}
	if proof.ReceiptID == "" || proof.CheckedAt < s.now().Unix()-30 || proof.CheckedAt > s.now().Unix()+5 {
		return Eligibility{}, access.ErrUnavailable
	}
	return proof, nil
}
func (s *Service) createSupport(ctx context.Context, tx *sql.Tx, p access.Principal, conversation, kind string, input BookInput, eligibilityReceipt string) (SupportRequest, bool, error) {
	if conversation == "" || len(conversation) > 128 {
		return SupportRequest{}, false, access.ErrInvalid
	}
	input.Topic = redaction.Content(strings.TrimSpace(input.Topic))
	input.PreferredTime = redaction.Content(strings.TrimSpace(input.PreferredTime))
	if !utf8.ValidString(input.Topic) || !utf8.ValidString(input.PreferredTime) || utf8.RuneCountInString(input.Topic) > 2000 || utf8.RuneCountInString(input.PreferredTime) > 200 {
		return SupportRequest{}, false, access.ErrInvalid
	}
	prior, err := scanSupport(tx.QueryRowContext(ctx, `SELECT `+supportColumns+` FROM lmm_assistant.platform_support WHERE user_id=$1 AND status IN('pending','accepted') FOR UPDATE`, p.UserID))
	if err == nil {
		if kind == "handoff" && prior.Kind == "appointment" && prior.Status == "pending" {
			prior, err = scanSupport(tx.QueryRowContext(ctx, `UPDATE lmm_assistant.platform_support SET kind='handoff',scheduled_at=0,preferred_time='',updated_at=$2,revision=revision+1 WHERE id=$1 RETURNING `+supportColumns, prior.ID, s.now().Unix()))
			if err != nil {
				return prior, false, access.ErrUnavailable
			}
		}
		return prior, false, nil
	}
	if !errors.Is(err, sql.ErrNoRows) {
		return prior, false, access.ErrUnavailable
	}
	if kind == "handoff" {
		input.ScheduledAt = 0
		input.PreferredTime = ""
	} else if kind == "appointment" {
		now := s.now().Unix()
		if input.Topic == "" || input.PreferredTime == "" || input.ScheduledAt <= now || input.ScheduledAt > now+90*24*3600 || eligibilityReceipt == "" {
			return prior, false, access.ErrInvalid
		}
	} else {
		return prior, false, access.ErrInvalid
	}
	now := s.now().Unix()
	request, err := scanSupport(tx.QueryRowContext(ctx, `INSERT INTO lmm_assistant.platform_support(user_id,core_conversation_id,kind,status,topic,preferred_time,scheduled_at,created_at,updated_at,eligibility_receipt) VALUES($1,$2,$3,'pending',$4,$5,$6,$7,$7,$8) RETURNING `+supportColumns, p.UserID, conversation, kind, input.Topic, input.PreferredTime, input.ScheduledAt, now, eligibilityReceipt))
	if err != nil {
		return request, false, access.ErrUnavailable
	}
	if _, err = tx.ExecContext(ctx, `INSERT INTO lmm_assistant.platform_support_todos(support_id,state,updated_at) VALUES($1,'pending',$2)`, request.ID, now); err != nil {
		return request, false, access.ErrUnavailable
	}
	return request, true, nil
}
func (s *Service) confirmHandoff(ctx context.Context, tx *sql.Tx, p access.Principal, raw json.RawMessage) (any, error) {
	var input handoffPayload
	if decode(raw, &input) != nil || utf8.RuneCountInString(input.Message) < 5 || utf8.RuneCountInString(input.Message) > 2000 {
		return nil, access.ErrInvalid
	}
	request, created, err := s.createSupport(ctx, tx, p, input.CoreConversationID, "handoff", BookInput{Topic: input.Message}, "")
	return map[string]any{"request": request, "created": created, "appointment_accepted": false}, err
}
func (s *Service) supportTool(ctx context.Context, tx *sql.Tx, in assistant.AgentInvocation, p access.Principal, session [32]byte) (assistant.AgentToolResult, bool, error) {
	switch in.Call.Scope.Name {
	case "get_human_support_status":
		var input struct{}
		if err := decode(in.Call.Arguments, &input); err != nil {
			return assistant.AgentToolResult{}, true, err
		}
		eligible, err := s.eligibility(ctx, in.Credential)
		if err != nil {
			return assistant.AgentToolResult{}, true, err
		}
		request, err := scanSupport(tx.QueryRowContext(ctx, `SELECT `+supportColumns+` FROM lmm_assistant.platform_support WHERE user_id=$1 AND status IN('pending','accepted')`, p.UserID))
		var active any
		if err == nil {
			active = request
		} else if !errors.Is(err, sql.ErrNoRows) {
			return assistant.AgentToolResult{}, true, access.ErrUnavailable
		}
		out, err := result(in.Call.OperationID, assistant.AgentToolSucceeded, map[string]any{"ok": true, "eligible": eligible.Eligible, "eligibility_receipt_id": eligible.ReceiptID, "request": active, "handoff_available": true, "current_time_utc": s.now().UTC().Format(time.RFC3339), "next_step": "A signed-in user can request a human handoff. Eligible users may explicitly request technical support with a topic, future date/time and timezone. An appointment request awaits administrator acceptance."}, "service:"+in.Call.OperationID)
		return out, true, err
	case "book_technical_support":
		var input BookInput
		if err := decode(in.Call.Arguments, &input); err != nil {
			return assistant.AgentToolResult{}, true, err
		}
		if !explicitBooking(in.OriginalUserText) {
			out, err := result(in.Call.OperationID, assistant.AgentToolFailed, map[string]any{"ok": false, "status": "explicit_request_required"}, "service:"+in.Call.OperationID)
			return out, true, err
		}
		eligible, err := s.eligibility(ctx, in.Credential)
		if err != nil {
			return assistant.AgentToolResult{}, true, err
		}
		if !eligible.Eligible {
			return assistant.AgentToolResult{}, true, access.ErrForbidden
		}
		request, created, err := s.createSupport(ctx, tx, p, in.Call.CoreConversationID, "appointment", input, eligible.ReceiptID)
		if err != nil {
			return assistant.AgentToolResult{}, true, err
		}
		out, err := result(in.Call.OperationID, assistant.AgentToolSucceeded, map[string]any{"ok": true, "created": created, "request": request, "appointment_accepted": false, "message": "Support request recorded in the administrator queue. The requested time awaits acceptance; an existing active request is not rescheduled."}, "service:"+in.Call.OperationID)
		return out, true, err
	case "request_human_support":
		var input struct {
			Message      string `json:"message"`
			Action       string `json:"action,omitempty"`
			TargetUserID *int64 `json:"target_user_id,omitempty"`
		}
		if err := decode(in.Call.Arguments, &input); err != nil {
			return assistant.AgentToolResult{}, true, err
		}
		if (input.Action != "" && input.Action != "support") || (input.TargetUserID != nil && *input.TargetUserID != p.UserID) {
			return assistant.AgentToolResult{}, true, access.ErrForbidden
		}
		input.Message = redaction.Content(strings.TrimSpace(input.Message))
		if utf8.RuneCountInString(input.Message) < 5 || utf8.RuneCountInString(input.Message) > 2000 {
			return assistant.AgentToolResult{}, true, access.ErrInvalid
		}
		payload, _ := json.Marshal(handoffPayload{input.Message, in.Call.CoreConversationID})
		out, err := s.prepareInTx(ctx, tx, in, p, session, payload)
		return out, true, err
	}
	return assistant.AgentToolResult{}, false, nil
}

// SupportQueue is an actual in-site administrator task queue. It sends no mail
// and exposes no transcript to an unassigned administrator.
func (s *Service) SupportQueue(ctx context.Context, credential string, before int64) ([]SupportRequest, error) {
	p, err := s.principal(ctx, credential)
	if err != nil {
		return nil, err
	}
	if p.Level < 5 {
		return nil, access.ErrForbidden
	}
	if before < 0 {
		return nil, access.ErrInvalid
	}
	rows, err := s.db.QueryContext(ctx, `SELECT `+supportColumns+` FROM lmm_assistant.platform_support WHERE status IN('pending','accepted') AND ($1::bigint=0 OR id<$1) ORDER BY id DESC LIMIT 30`, before)
	if err != nil {
		return nil, access.ErrUnavailable
	}
	defer rows.Close()
	out := []SupportRequest{}
	for rows.Next() {
		request, err := scanSupport(rows)
		if err != nil {
			return nil, access.ErrUnavailable
		}
		out = append(out, request)
	}
	if rows.Err() != nil {
		return nil, access.ErrUnavailable
	}
	return out, nil
}
func (s *Service) AcceptSupport(ctx context.Context, credential string, id int64) (SupportRequest, error) {
	p, err := s.principal(ctx, credential)
	if err != nil {
		return SupportRequest{}, err
	}
	if p.Level < 5 || id <= 0 {
		return SupportRequest{}, access.ErrForbidden
	}
	tx, err := s.db.BeginTx(ctx, nil)
	if err != nil {
		return SupportRequest{}, access.ErrUnavailable
	}
	defer tx.Rollback()
	request, err := scanSupport(tx.QueryRowContext(ctx, `SELECT `+supportColumns+` FROM lmm_assistant.platform_support WHERE id=$1 FOR UPDATE`, id))
	if errors.Is(err, sql.ErrNoRows) {
		return request, access.ErrNotFound
	}
	if err != nil {
		return request, access.ErrUnavailable
	}
	if request.Status == "accepted" && request.AssignedAdminID != nil && *request.AssignedAdminID == p.UserID {
		return request, nil
	}
	if request.Status != "pending" {
		return request, access.ErrConflict
	}
	now := s.now().Unix()
	request, err = scanSupport(tx.QueryRowContext(ctx, `UPDATE lmm_assistant.platform_support SET status='accepted',assigned_admin_id=$2,accepted_at=$3,updated_at=$3,revision=revision+1 WHERE id=$1 RETURNING `+supportColumns, id, p.UserID, now))
	if err != nil {
		return request, access.ErrUnavailable
	}
	if _, err = tx.ExecContext(ctx, `UPDATE lmm_assistant.platform_support_todos SET state='assigned',assigned_admin_id=$2,updated_at=$3 WHERE support_id=$1`, id, p.UserID, now); err != nil {
		return request, access.ErrUnavailable
	}
	if tx.Commit() != nil {
		return request, access.ErrUnavailable
	}
	return request, nil
}
func (s *Service) CloseSupport(ctx context.Context, credential string, id int64, cancel bool) (SupportRequest, error) {
	p, err := s.principal(ctx, credential)
	if err != nil {
		return SupportRequest{}, err
	}
	tx, err := s.db.BeginTx(ctx, nil)
	if err != nil {
		return SupportRequest{}, access.ErrUnavailable
	}
	defer tx.Rollback()
	request, err := scanSupport(tx.QueryRowContext(ctx, `SELECT `+supportColumns+` FROM lmm_assistant.platform_support WHERE id=$1 FOR UPDATE`, id))
	if errors.Is(err, sql.ErrNoRows) {
		return request, access.ErrNotFound
	}
	if err != nil {
		return request, access.ErrUnavailable
	}
	owner := request.UserID == p.UserID
	assigned := p.Level >= 5 && request.AssignedAdminID != nil && *request.AssignedAdminID == p.UserID
	if (!owner && !assigned) || (owner && !cancel) {
		return request, access.ErrForbidden
	}
	if request.ClosedAt != nil {
		return request, nil
	}
	status := "completed"
	if cancel {
		status = "cancelled"
	}
	now := s.now().Unix()
	request, err = scanSupport(tx.QueryRowContext(ctx, `UPDATE lmm_assistant.platform_support SET status=$2,closed_at=$3,updated_at=$3,closed_message_id=(SELECT max(id) FROM lmm_assistant.platform_support_messages WHERE support_id=$1),revision=revision+1 WHERE id=$1 RETURNING `+supportColumns, id, status, now))
	if err != nil {
		return request, access.ErrUnavailable
	}
	if _, err = tx.ExecContext(ctx, `UPDATE lmm_assistant.platform_support_todos SET state='closed',updated_at=$2 WHERE support_id=$1`, id, now); err != nil {
		return request, access.ErrUnavailable
	}
	if tx.Commit() != nil {
		return request, access.ErrUnavailable
	}
	return request, nil
}
