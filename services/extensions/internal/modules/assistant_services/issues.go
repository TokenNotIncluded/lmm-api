// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later
// Domain rules adapted from fixed main model/assistant_workspace.go.
package assistant_services

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"strings"
	"unicode/utf8"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/redaction"
)

type Issue struct {
	ID            int64  `json:"id"`
	ReporterID    int64  `json:"reporter_id"`
	Kind          string `json:"kind"`
	Title         string `json:"title"`
	Body          string `json:"body"`
	Visibility    string `json:"visibility"`
	Status        string `json:"status"`
	Revision      int64  `json:"revision"`
	CreatedAt     int64  `json:"created_at"`
	UpdatedAt     int64  `json:"updated_at"`
	BodyTruncated bool   `json:"body_truncated,omitempty"`
}
type IssueEvent struct {
	ID            int64  `json:"id"`
	IssueID       int64  `json:"issue_id"`
	Visibility    string `json:"visibility"`
	Status        string `json:"status"`
	Note          string `json:"note"`
	CreatedAt     int64  `json:"created_at"`
	NoteTruncated bool   `json:"note_truncated,omitempty"`
}
type IssueInput struct {
	Kind       string `json:"kind"`
	Title      string `json:"title"`
	Body       string `json:"body"`
	Visibility string `json:"visibility"`
}
type IssueUpdate struct {
	ID         int64  `json:"issue_id"`
	Revision   int64  `json:"revision"`
	Status     string `json:"status"`
	Visibility string `json:"visibility"`
	Note       string `json:"note"`
}

func normalizeIssue(in IssueInput) (IssueInput, error) {
	if in.Visibility == "" {
		in.Visibility = "admin"
	}
	switch in.Kind {
	case "bug", "security", "experience", "feature":
	default:
		return in, access.ErrInvalid
	}
	if in.Kind == "security" {
		in.Visibility = "admin"
	}
	if in.Visibility != "user" && in.Visibility != "admin" {
		return in, access.ErrInvalid
	}
	in.Title = strings.TrimSpace(redaction.Content(in.Title))
	in.Body = strings.TrimSpace(redaction.Content(in.Body))
	if !utf8.ValidString(in.Title) || !utf8.ValidString(in.Body) || utf8.RuneCountInString(in.Title) < 2 || utf8.RuneCountInString(in.Title) > 128 || utf8.RuneCountInString(in.Body) < 5 || utf8.RuneCountInString(in.Body) > 4000 {
		return in, access.ErrInvalid
	}
	return in, nil
}
func normalizeIssueUpdate(in IssueUpdate) (IssueUpdate, error) {
	if in.ID <= 0 || in.Revision < 1 {
		return in, access.ErrInvalid
	}
	switch in.Status {
	case "open", "triaged", "in_progress", "resolved", "declined":
	default:
		return in, access.ErrInvalid
	}
	if in.Visibility != "user" && in.Visibility != "admin" {
		return in, access.ErrInvalid
	}
	in.Note = strings.TrimSpace(redaction.Content(in.Note))
	if !utf8.ValidString(in.Note) || utf8.RuneCountInString(in.Note) > 1500 {
		return in, access.ErrInvalid
	}
	return in, nil
}

const issueColumns = `id,reporter_id,kind,title,body,visibility,status,revision,created_at,updated_at`

func scanIssue(row interface{ Scan(...any) error }) (Issue, error) {
	var issue Issue
	err := row.Scan(&issue.ID, &issue.ReporterID, &issue.Kind, &issue.Title, &issue.Body, &issue.Visibility, &issue.Status, &issue.Revision, &issue.CreatedAt, &issue.UpdatedAt)
	if err == nil {
		issue.Title = redaction.Content(issue.Title)
		issue.Body = redaction.Content(issue.Body)
	}
	return issue, err
}
func (s *Service) readIssues(ctx context.Context, tx *sql.Tx, p access.Principal, id, before int64) (any, error) {
	if id < 0 || before < 0 {
		return nil, access.ErrInvalid
	}
	if id > 0 {
		issue, err := scanIssue(tx.QueryRowContext(ctx, `SELECT `+issueColumns+` FROM lmm_assistant.site_issues WHERE id=$1 AND ($2 OR (reporter_id=$3 AND visibility='user'))`, id, p.Level >= 5, p.UserID))
		if errors.Is(err, sql.ErrNoRows) {
			return nil, access.ErrNotFound
		}
		if err != nil {
			return nil, access.ErrUnavailable
		}
		events, err := tx.QueryContext(ctx, `SELECT id,issue_id,visibility,status,note,created_at FROM lmm_assistant.site_issue_events WHERE issue_id=$1 AND ($2 OR visibility='user') ORDER BY id DESC LIMIT 30`, id, p.Level >= 5)
		if err != nil {
			return nil, access.ErrUnavailable
		}
		defer events.Close()
		out := []IssueEvent{}
		for events.Next() {
			var event IssueEvent
			if events.Scan(&event.ID, &event.IssueID, &event.Visibility, &event.Status, &event.Note, &event.CreatedAt) != nil {
				return nil, access.ErrUnavailable
			}
			event = boundedIssueEvent(event)
			out = append(out, event)
		}
		if events.Err() != nil {
			return nil, access.ErrUnavailable
		}
		return map[string]any{"ok": true, "issue": issue, "events": out}, nil
	}
	rows, err := tx.QueryContext(ctx, `SELECT `+issueColumns+` FROM lmm_assistant.site_issues WHERE ($1 OR (reporter_id=$2 AND visibility='user')) AND ($3::bigint=0 OR id<$3) ORDER BY id DESC LIMIT 21`, p.Level >= 5, p.UserID, before)
	if err != nil {
		return nil, access.ErrUnavailable
	}
	defer rows.Close()
	out := []Issue{}
	hasMore := false
	for rows.Next() {
		if len(out) == 20 {
			hasMore = true
			break
		}
		issue, err := scanIssue(rows)
		if err != nil {
			return nil, access.ErrUnavailable
		}
		if utf8.RuneCountInString(issue.Body) > 250 {
			issue.Body = safeText(issue.Body, 250)
			issue.BodyTruncated = true
		}
		out = append(out, issue)
	}
	if rows.Err() != nil {
		return nil, access.ErrUnavailable
	}
	var cursor int64
	if len(out) > 0 {
		cursor = out[len(out)-1].ID
	}
	return map[string]any{"ok": true, "issues": out, "has_more": hasMore, "next_before_id": cursor, "list_bodies_may_be_truncated": true}, nil
}
func (s *Service) createIssue(ctx context.Context, tx *sql.Tx, p access.Principal, raw json.RawMessage) (any, error) {
	var input IssueInput
	if decode(raw, &input) != nil {
		return nil, access.ErrInvalid
	}
	input, err := normalizeIssue(input)
	if err != nil {
		return nil, err
	}
	now := s.now().Unix()
	issue, err := scanIssue(tx.QueryRowContext(ctx, `INSERT INTO lmm_assistant.site_issues(reporter_id,kind,title,body,visibility,status,revision,created_at,updated_at) VALUES($1,$2,$3,$4,$5,'open',1,$6,$6) RETURNING `+issueColumns, p.UserID, input.Kind, input.Title, input.Body, input.Visibility, now))
	if err != nil {
		return nil, access.ErrUnavailable
	}
	if _, err = tx.ExecContext(ctx, `INSERT INTO lmm_assistant.site_issue_events(issue_id,actor_id,visibility,status,note,created_at) VALUES($1,$2,$3,'open','',$4)`, issue.ID, p.UserID, issue.Visibility, now); err != nil {
		return nil, access.ErrUnavailable
	}
	return issue, nil
}
func (s *Service) updateIssue(ctx context.Context, tx *sql.Tx, p access.Principal, raw json.RawMessage) (any, error) {
	if p.Level < 5 {
		return nil, access.ErrForbidden
	}
	var input IssueUpdate
	if decode(raw, &input) != nil {
		return nil, access.ErrInvalid
	}
	input, err := normalizeIssueUpdate(input)
	if err != nil {
		return nil, err
	}
	issue, err := scanIssue(tx.QueryRowContext(ctx, `SELECT `+issueColumns+` FROM lmm_assistant.site_issues WHERE id=$1 FOR UPDATE`, input.ID))
	if errors.Is(err, sql.ErrNoRows) {
		return nil, access.ErrNotFound
	}
	if err != nil {
		return nil, access.ErrUnavailable
	}
	if issue.Revision != input.Revision {
		return nil, access.ErrConflict
	}
	if issue.Kind == "security" {
		input.Visibility = "admin"
	}
	now := s.now().Unix()
	issue, err = scanIssue(tx.QueryRowContext(ctx, `UPDATE lmm_assistant.site_issues SET status=$2,visibility=$3,revision=revision+1,updated_at=$4 WHERE id=$1 RETURNING `+issueColumns, input.ID, input.Status, input.Visibility, now))
	if err != nil {
		return nil, access.ErrUnavailable
	}
	if _, err = tx.ExecContext(ctx, `INSERT INTO lmm_assistant.site_issue_events(issue_id,actor_id,visibility,status,note,created_at) VALUES($1,$2,$3,$4,$5,$6)`, issue.ID, p.UserID, issue.Visibility, issue.Status, input.Note, now); err != nil {
		return nil, access.ErrUnavailable
	}
	return issue, nil
}
func (s *Service) issueTool(ctx context.Context, tx *sql.Tx, in assistant.AgentInvocation, p access.Principal, session [32]byte) (assistant.AgentToolResult, bool, error) {
	switch in.Call.Scope.Name {
	case "get_site_issues":
		var input struct {
			ID     int64 `json:"issue_id"`
			Before int64 `json:"before_id"`
		}
		if err := decode(in.Call.Arguments, &input); err != nil {
			return assistant.AgentToolResult{}, true, err
		}
		value, err := s.readIssues(ctx, tx, p, input.ID, input.Before)
		if err != nil {
			return assistant.AgentToolResult{}, true, err
		}
		out, err := result(in.Call.OperationID, assistant.AgentToolSucceeded, value, "service:"+in.Call.OperationID)
		return out, true, err
	case "create_site_issue":
		var input IssueInput
		if err := decode(in.Call.Arguments, &input); err != nil {
			return assistant.AgentToolResult{}, true, err
		}
		input, err := normalizeIssue(input)
		if err != nil {
			return assistant.AgentToolResult{}, true, err
		}
		payload, _ := json.Marshal(input)
		out, err := s.prepareInTx(ctx, tx, in, p, session, payload)
		return out, true, err
	case "update_site_issue":
		if p.Level < 5 {
			return assistant.AgentToolResult{}, true, access.ErrForbidden
		}
		var input IssueUpdate
		if err := decode(in.Call.Arguments, &input); err != nil {
			return assistant.AgentToolResult{}, true, err
		}
		input, err := normalizeIssueUpdate(input)
		if err != nil {
			return assistant.AgentToolResult{}, true, err
		}
		issue, err := scanIssue(tx.QueryRowContext(ctx, `SELECT `+issueColumns+` FROM lmm_assistant.site_issues WHERE id=$1`, input.ID))
		if errors.Is(err, sql.ErrNoRows) {
			return assistant.AgentToolResult{}, true, access.ErrNotFound
		}
		if err != nil {
			return assistant.AgentToolResult{}, true, access.ErrUnavailable
		}
		if issue.Revision != input.Revision {
			return assistant.AgentToolResult{}, true, access.ErrConflict
		}
		if issue.Kind == "security" {
			input.Visibility = "admin"
		}
		payload, _ := json.Marshal(input)
		out, err := s.prepareInTx(ctx, tx, in, p, session, payload)
		return out, true, err
	}
	return assistant.AgentToolResult{}, false, nil
}
func (s *Service) defaultConfirmations() map[string]confirmationAction {
	return map[string]confirmationAction{"create_site_issue": {minimumLevel: 0, apply: s.createIssue}, "update_site_issue": {minimumLevel: 5, apply: s.updateIssue}, "request_human_support": {minimumLevel: 0, apply: s.confirmHandoff}}
}

// Escaped ASCII can use six JSON bytes per rune. Keep a full issue body while
// bounding every historical note; truncation is explicit in the tool result.
func boundedIssueEvent(event IssueEvent) IssueEvent {
	event.Note = redaction.Content(event.Note)
	if utf8.RuneCountInString(event.Note) > 150 {
		event.Note = safeText(event.Note, 150)
		event.NoteTruncated = true
	}
	return event
}
