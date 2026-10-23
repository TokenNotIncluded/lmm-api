// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later
// Limits, ranking and safe profile strategies adapted from fixed main
// 3948a0a1 model/assistant_memory.go and controller/assistant_skill_tools.go.
package assistant_services

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"sort"
	"strings"
	"unicode"
	"unicode/utf8"

	"github.com/TokenNotIncluded/lmm-api/services/extensions/internal/modules/assistant"
	"github.com/TokenNotIncluded/lmm-api/services/extensions/internal/modules/assistant/access"
	"github.com/TokenNotIncluded/lmm-api/services/extensions/internal/modules/assistant/redaction"
)

type Memory struct {
	ID        int64    `json:"id"`
	Title     string   `json:"title"`
	Content   string   `json:"content"`
	Tags      []string `json:"tags"`
	Source    string   `json:"source"`
	Enabled   bool     `json:"enabled"`
	CreatedAt int64    `json:"created_at"`
	UpdatedAt int64    `json:"updated_at"`
}
type MemoryInput struct {
	Title   string   `json:"title"`
	Content string   `json:"content"`
	Tags    []string `json:"tags"`
}

func normalizedText(value string, max int) (string, error) {
	if !utf8.ValidString(value) {
		return "", access.ErrInvalid
	}
	value = strings.Map(func(r rune) rune {
		if unicode.IsControl(r) || unicode.In(r, unicode.Cf) {
			return -1
		}
		return r
	}, strings.TrimSpace(value))
	value = redaction.Content(strings.Join(strings.Fields(value), " "))
	if value == "" || utf8.RuneCountInString(value) > max {
		return "", access.ErrInvalid
	}
	return value, nil
}
func normalizeMemory(input MemoryInput) (MemoryInput, error) {
	var err error
	input.Title, err = normalizedText(input.Title, 80)
	if err != nil {
		return input, err
	}
	input.Content, err = normalizedText(input.Content, 800)
	if err != nil {
		return input, err
	}
	if len(input.Tags) > 8 {
		return input, access.ErrInvalid
	}
	tags := []string{}
	seen := map[string]bool{}
	for _, tag := range input.Tags {
		tag, err = normalizedText(tag, 48)
		if err != nil {
			return input, err
		}
		tag = strings.ToLower(tag)
		if !seen[tag] {
			tags = append(tags, tag)
			seen[tag] = true
		}
	}
	input.Tags = tags
	return input, nil
}
func scanMemory(row interface{ Scan(...any) error }) (Memory, error) {
	var memory Memory
	var raw []byte
	err := row.Scan(&memory.ID, &memory.Title, &memory.Content, &raw, &memory.Source, &memory.Enabled, &memory.CreatedAt, &memory.UpdatedAt)
	if err != nil {
		return memory, err
	}
	if json.Unmarshal(raw, &memory.Tags) != nil {
		return memory, access.ErrUnavailable
	}
	normalized, err := normalizeMemory(MemoryInput{Title: memory.Title, Content: memory.Content, Tags: memory.Tags})
	if err != nil {
		return memory, access.ErrUnavailable
	}
	memory.Title, memory.Content, memory.Tags = normalized.Title, normalized.Content, normalized.Tags
	return memory, nil
}
func (s *Service) remember(ctx context.Context, tx *sql.Tx, user int64, input MemoryInput) (Memory, error) {
	input, err := normalizeMemory(input)
	if err != nil {
		return Memory{}, err
	}
	var id int64
	err = tx.QueryRowContext(ctx, `SELECT id FROM lmm_assistant.memories WHERE user_id=$1 AND title=$2 FOR UPDATE`, user, input.Title).Scan(&id)
	if errors.Is(err, sql.ErrNoRows) {
		var count int
		if tx.QueryRowContext(ctx, `SELECT count(*) FROM lmm_assistant.memories WHERE user_id=$1`, user).Scan(&count) != nil {
			return Memory{}, access.ErrUnavailable
		}
		if count >= 64 {
			return Memory{}, access.ErrLimit
		}
	} else if err != nil {
		return Memory{}, access.ErrUnavailable
	}
	tags, _ := json.Marshal(input.Tags)
	now := s.now().Unix()
	row := tx.QueryRowContext(ctx, `INSERT INTO lmm_assistant.memories(user_id,title,content,tags,source,enabled,updated_by,created_at,updated_at) VALUES($1,$2,$3,$4,'assistant',true,$1,$5,$5) ON CONFLICT(user_id,title) DO UPDATE SET content=EXCLUDED.content,tags=EXCLUDED.tags,source='assistant',enabled=true,updated_by=EXCLUDED.updated_by,updated_at=EXCLUDED.updated_at RETURNING id,title,content,tags,source,enabled,created_at,updated_at`, user, input.Title, input.Content, string(tags), now)
	memory, err := scanMemory(row)
	if err != nil {
		return memory, access.ErrUnavailable
	}
	return memory, nil
}
func (s *Service) recall(ctx context.Context, tx *sql.Tx, user int64, query string, limit int) ([]Memory, error) {
	if !utf8.ValidString(query) || utf8.RuneCountInString(query) > 240 || limit < 0 || limit > 4 {
		return nil, access.ErrInvalid
	}
	if limit == 0 {
		limit = 4
	}
	rows, err := tx.QueryContext(ctx, `SELECT id,title,content,tags,source,enabled,created_at,updated_at FROM lmm_assistant.memories WHERE user_id=$1 AND enabled ORDER BY updated_at DESC,id DESC LIMIT 64`, user)
	if err != nil {
		return nil, access.ErrUnavailable
	}
	defer rows.Close()
	type scored struct {
		memory Memory
		score  int
	}
	matches := []scored{}
	terms := strings.Fields(strings.ToLower(strings.TrimSpace(query)))
	if len(terms) > 12 {
		terms = terms[:12]
	}
	for rows.Next() {
		memory, err := scanMemory(rows)
		if err != nil {
			return nil, access.ErrUnavailable
		}
		score := 0
		tags, _ := json.Marshal(memory.Tags)
		for _, term := range terms {
			if strings.Contains(strings.ToLower(memory.Title), term) {
				score += 4
			}
			if strings.Contains(strings.ToLower(string(tags)), term) {
				score += 2
			}
			if strings.Contains(strings.ToLower(memory.Content), term) {
				score++
			}
		}
		if len(terms) == 0 || score > 0 {
			matches = append(matches, scored{memory, score})
		}
	}
	if rows.Err() != nil {
		return nil, access.ErrUnavailable
	}
	sort.SliceStable(matches, func(i, j int) bool {
		if matches[i].score != matches[j].score {
			return matches[i].score > matches[j].score
		}
		return matches[i].memory.UpdatedAt > matches[j].memory.UpdatedAt
	})
	if len(matches) > limit {
		matches = matches[:limit]
	}
	out := make([]Memory, 0, len(matches))
	for _, match := range matches {
		out = append(out, match.memory)
	}
	return out, nil
}

type ProfileInput struct {
	Key  string   `json:"profile_key"`
	Tags []string `json:"tags"`
}

var profileStrategies = map[string]string{
	"technical_cost_sensitive": "Answer the current technical question directly. Prefer exact model IDs, endpoints, commands, and verifiable details; avoid beginner screening and payment pressure.",
	"guided_buyer":             "Use short numbered steps, explain one decision at a time, and ask only the minimum question needed for the next action.",
	"production_operator":      "Prioritize reliability, operational checks, concurrency, observability, rollback, and concise production-ready actions.",
	"privacy_conscious":        "Prefer data minimization, local options, explicit privacy boundaries, and never request credentials or unnecessary personal data.",
	"mobile_accessibility":     "Prefer short mobile-friendly actions, clear labels, keyboard and screen-reader compatible instructions, and avoid dense layouts.",
	"normal_user":              "Answer the user's current request directly and concisely without repeating generic onboarding questions.",
	"support_seeking":          "First resolve the immediate support issue, summarize the evidence, and prepare a human handoff only when needed.",
	"l0_applicant":             "Answer directly. Reuse the stated ordinary purpose and enable L1 with get_registration_risk and grant_l1_access immediately when allowed, even in the first reply. No turn minimum or application letter. On failure give the actual next step or human support, never require more chatting. Developer writes remain unavailable until the grant succeeds.",
}
var profileTags = map[string]bool{"advanced": true, "needs_steps": true, "cost_sensitive": true, "production": true, "privacy": true, "mobile": true, "accessibility": true, "support": true, "l0": true}

func (s *Service) rememberProfile(ctx context.Context, tx *sql.Tx, user int64, input ProfileInput) (any, error) {
	strategy, ok := profileStrategies[input.Key]
	if !ok || len(input.Tags) > 4 {
		return nil, access.ErrInvalid
	}
	tags := []string{}
	seen := map[string]bool{}
	for _, tag := range input.Tags {
		if !profileTags[tag] {
			return nil, access.ErrInvalid
		}
		if !seen[tag] {
			tags = append(tags, tag)
			seen[tag] = true
		}
	}
	var source string
	err := tx.QueryRowContext(ctx, `SELECT source FROM lmm_assistant.profiles WHERE user_id=$1 FOR UPDATE`, user).Scan(&source)
	if err == nil && source != "assistant" {
		return map[string]any{"ok": false, "status": "administrator_managed", "error": "profile skill is managed by an administrator and was not changed"}, nil
	}
	if err != nil && !errors.Is(err, sql.ErrNoRows) {
		return nil, access.ErrUnavailable
	}
	raw, _ := json.Marshal(tags)
	updated, err := tx.ExecContext(ctx, `INSERT INTO lmm_assistant.profiles(user_id,profile_key,tags,strategy,source,enabled,updated_by,created_at,updated_at) VALUES($1,$2,$3,$4,'assistant',true,$1,$5,$5) ON CONFLICT(user_id) DO UPDATE SET profile_key=EXCLUDED.profile_key,tags=EXCLUDED.tags,strategy=EXCLUDED.strategy,enabled=true,updated_by=EXCLUDED.updated_by,updated_at=EXCLUDED.updated_at WHERE lmm_assistant.profiles.source='assistant'`, user, input.Key, string(raw), strategy, s.now().Unix())
	if err != nil {
		return nil, access.ErrUnavailable
	}
	count, err := updated.RowsAffected()
	if err != nil {
		return nil, access.ErrUnavailable
	}
	if count != 1 {
		return map[string]any{"ok": false, "status": "administrator_managed"}, nil
	}
	return map[string]any{"ok": true, "status": "profile_skill_saved", "owner_scope": "signed_in_user_only", "profile_key": input.Key, "tags": tags}, nil
}
func (s *Service) forgetProfile(ctx context.Context, tx *sql.Tx, user int64, confirm bool, original string) (any, error) {
	if !confirm || !explicitProfileForget(original) {
		return map[string]any{"ok": false, "status": "explicit_request_required", "error": "profile removal requires an explicit user request"}, nil
	}
	var source string
	err := tx.QueryRowContext(ctx, `SELECT source FROM lmm_assistant.profiles WHERE user_id=$1 FOR UPDATE`, user).Scan(&source)
	if errors.Is(err, sql.ErrNoRows) {
		return map[string]any{"ok": false, "status": "not_found"}, nil
	}
	if err != nil {
		return nil, access.ErrUnavailable
	}
	if source != "assistant" {
		return map[string]any{"ok": false, "status": "administrator_managed"}, nil
	}
	if _, err = tx.ExecContext(ctx, `DELETE FROM lmm_assistant.profiles WHERE user_id=$1 AND source='assistant'`, user); err != nil {
		return nil, access.ErrUnavailable
	}
	return map[string]any{"ok": true, "status": "profile_skill_forgotten", "owner_scope": "signed_in_user_only"}, nil
}

// Only the immutable original user request is considered. Quoted/instructional
// tool text is removed before testing explicit deletion intent.
func explicitProfileForget(original string) bool { return profileForgetIntent(original) }
func (s *Service) memoryTool(ctx context.Context, tx *sql.Tx, in assistant.AgentInvocation, p access.Principal) (any, bool, error) {
	switch in.Call.Scope.Name {
	case "recall_memory":
		var input struct {
			Query string `json:"query"`
			Limit int    `json:"limit"`
		}
		if err := decode(in.Call.Arguments, &input); err != nil {
			return nil, true, err
		}
		rows, err := s.recall(ctx, tx, p.UserID, input.Query, input.Limit)
		return map[string]any{"ok": true, "owner_scope": "signed_in_user_only", "memories": rows, "match_count": len(rows)}, true, err
	case "remember_memory":
		var input MemoryInput
		if err := decode(in.Call.Arguments, &input); err != nil {
			return nil, true, err
		}
		memory, err := s.remember(ctx, tx, p.UserID, input)
		return map[string]any{"ok": true, "status": "remembered", "owner_scope": "signed_in_user_only", "memory": memory}, true, err
	case "remember_profile_skill":
		var input ProfileInput
		if err := decode(in.Call.Arguments, &input); err != nil {
			return nil, true, err
		}
		value, err := s.rememberProfile(ctx, tx, p.UserID, input)
		return value, true, err
	case "forget_profile_skill":
		var input struct {
			Confirm bool `json:"confirm"`
		}
		if err := decode(in.Call.Arguments, &input); err != nil {
			return nil, true, err
		}
		value, err := s.forgetProfile(ctx, tx, p.UserID, input.Confirm, in.OriginalUserText)
		return value, true, err
	}
	return nil, false, nil
}
