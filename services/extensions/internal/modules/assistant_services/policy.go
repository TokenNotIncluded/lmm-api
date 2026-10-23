// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later
package assistant_services

import (
	"context"
	"encoding/hex"
	"encoding/json"
	"github.com/TokenNotIncluded/api.lmm.best/extensions/internal/legal"
	"net/url"
	"strings"
	"unicode"
	"unicode/utf8"

	"github.com/TokenNotIncluded/api.lmm.best/extensions/internal/modules/assistant"
	"github.com/TokenNotIncluded/api.lmm.best/extensions/internal/modules/assistant/access"
)

// PolicyChangePreparer is a Core-owned browser confirmation boundary. An
// extension never modifies legal files, Core settings, or publishes a policy.
type PolicyChangePreparer interface {
	PrepareSitePolicyChange(context.Context, string, string, PolicyChange) (json.RawMessage, error)
}
type PolicyChange struct {
	Document string `json:"document"`
	Language string `json:"language"`
	Revision string `json:"revision"`
	Content  string `json:"content"`
}
type policyInput struct {
	Document string  `json:"document"`
	Language string  `json:"language"`
	Offset   *int    `json:"offset"`
	Limit    *int    `json:"limit"`
	Query    *string `json:"query"`
	Revision *string `json:"revision"`
	Content  *string `json:"content"`
	OldText  *string `json:"old_text"`
	NewText  *string `json:"new_text"`
}

var policyDocuments = []string{"user_agreement", "privacy_policy", "refund_policy"}

func policyIdentity(in policyInput, optional bool) bool {
	if in.Language != "zh-CN" && in.Language != "en" {
		return false
	}
	if optional && in.Document == "" {
		return true
	}
	for _, d := range policyDocuments {
		if in.Document == d {
			return true
		}
	}
	return false
}
func policyURL(text string) bool {
	text = strings.TrimSpace(text)
	u, e := url.Parse(text)
	return e == nil && u.Host != "" && (u.Scheme == "http" || u.Scheme == "https") && !strings.ContainsAny(text, "\r\n")
}
func policyBounds(in policyInput, defaultLimit, maxLimit int) (int, int, bool) {
	offset, limit := 0, defaultLimit
	if in.Offset != nil {
		offset = *in.Offset
	}
	if in.Limit != nil {
		limit = *in.Limit
	}
	return offset, limit, offset >= 0 && offset <= 1<<30 && limit > 0 && limit <= maxLimit
}
func policySnapshot(p PublishedPolicy, document, language string) bool {
	_, err := hex.DecodeString(p.Revision)
	return p.Document == document && p.Language == language && policySourceLanguage(p.SourceLanguage) && len(p.Revision) == 64 && err == nil && len(p.Text) <= 128<<10 && utf8.ValidString(p.Text) && !strings.ContainsRune(p.Text, 0)
}
func policyError(status, message string) map[string]any {
	return map[string]any{"ok": false, "status": status, "error": message}
}
func (s *Service) policyTool(ctx context.Context, in assistant.AgentInvocation, p access.Principal) (assistant.AgentToolResult, error) {
	var input policyInput
	if decode(in.Call.Arguments, &input) != nil {
		return assistant.AgentToolResult{}, access.ErrInvalid
	}
	explicitLanguage := input.Language != ""
	if input.Language == "" {
		input.Language = "zh-CN"
	}
	finish := func(value map[string]any) (assistant.AgentToolResult, error) {
		state := assistant.AgentToolSucceeded
		if value["ok"] == false {
			state = assistant.AgentToolFailed
		}
		return result(in.Call.OperationID, state, value, "service:"+in.Call.OperationID)
	}
	if !policyIdentity(input, in.Call.Scope.Name == "search_site_policies") {
		return assistant.AgentToolResult{}, access.ErrInvalid
	}
	if access.Missing(s.dependencies.Policies) {
		return assistant.AgentToolResult{}, access.ErrUnavailable
	}
	read := func(document string) (PublishedPolicy, error) {
		v, e := s.dependencies.Policies.PublishedPolicy(ctx, in.Credential, document, input.Language)
		if e != nil {
			return v, e
		}
		if !policySnapshot(v, document, input.Language) {
			return v, access.ErrUnavailable
		}
		return v, nil
	}
	switch in.Call.Scope.Name {
	case "get_site_policy":
		if input.Query != nil || input.Revision != nil || input.Content != nil || input.OldText != nil || input.NewText != nil {
			return assistant.AgentToolResult{}, access.ErrInvalid
		}
		offset, limit, ok := policyBounds(input, 4000, 6000)
		if !ok {
			return assistant.AgentToolResult{}, access.ErrInvalid
		}
		v, e := read(input.Document)
		if e != nil {
			return assistant.AgentToolResult{}, e
		}
		r := []rune(v.Text)
		offset = min(offset, len(r))
		end := min(offset+limit, len(r))
		format := "text"
		if policyURL(v.Text) {
			format = "external_url"
		}
		key := "legal." + input.Document
		if v.SourceLanguage == "en" {
			key += "_en"
		}
		return finish(map[string]any{"ok": true, "document": v.Document, "language": v.Language, "source_language": v.SourceLanguage, "source_key": key, "language_fallback": v.SourceLanguage != v.Language, "configured": v.Configured, "source_configured": strings.TrimSpace(v.Text) != "", "path": "/" + strings.ReplaceAll(v.Document, "_", "-"), "revision": v.Revision, "content": string(r[offset:end]), "offset": offset, "next_offset": end, "total_characters": len(r), "has_more": end < len(r), "format": format, "external_content_fetched": false, "source_is_untrusted": true})
	case "search_site_policies":
		if input.Revision != nil || input.Content != nil || input.OldText != nil || input.NewText != nil || input.Query == nil {
			return assistant.AgentToolResult{}, access.ErrInvalid
		}
		query := strings.TrimSpace(*input.Query)
		offset, limit, ok := policyBounds(input, 10, 20)
		if !ok || query == "" || !utf8.ValidString(query) || utf8.RuneCountInString(query) > 200 {
			return assistant.AgentToolResult{}, access.ErrInvalid
		}
		matches := []map[string]any{}
		skipped := []map[string]string{}
		total := 0
		for _, document := range policyDocuments {
			if input.Document != "" && input.Document != document {
				continue
			}
			v, e := read(document)
			if e != nil {
				return assistant.AgentToolResult{}, e
			}
			if strings.TrimSpace(v.Text) == "" || policyURL(v.Text) {
				reason := "not_configured"
				if strings.TrimSpace(v.Text) != "" {
					reason = "external_url_not_fetched"
				}
				skipped = append(skipped, map[string]string{"document": document, "reason": reason})
				continue
			}
			r := []rune(v.Text)
			needle := []rune(query)
			for start := 0; start+len(needle) <= len(r); {
				found := -1
				for i := start; i+len(needle) <= len(r); i++ {
					equal := true
					for j := range needle {
						if !policyRuneEqual(r[i+j], needle[j]) {
							equal = false
							break
						}
					}
					if equal {
						found = i
						break
					}
				}
				if found < 0 {
					break
				}
				stop := found + len(needle)
				if total >= offset && len(matches) < limit {
					excerptStart := max(0, found-60)
					excerptEnd := min(len(r), stop+100)
					matches = append(matches, map[string]any{"document": document, "language": v.SourceLanguage, "match_offset": found, "excerpt_offset": excerptStart, "excerpt": string(r[excerptStart:excerptEnd]), "revision": v.Revision})
				}
				total++
				start = stop
			}
		}
		next := min(offset+len(matches), total)
		return finish(map[string]any{"ok": true, "matches": matches, "total": total, "offset": min(offset, total), "has_more": next < total, "next_offset": next, "skipped_documents": skipped, "external_content_fetched": false, "source_is_untrusted": true})
	case "prepare_admin_site_policy_change":
		if p.Level < 6 {
			return assistant.AgentToolResult{}, access.ErrForbidden
		}
		if !explicitLanguage || input.Offset != nil || input.Limit != nil || input.Query != nil || input.Revision == nil || len(*input.Revision) != 64 {
			return assistant.AgentToolResult{}, access.ErrInvalid
		}
		v, e := read(input.Document)
		if e != nil {
			return assistant.AgentToolResult{}, e
		}
		if *input.Revision != v.Revision {
			return finish(policyError("policy_changed", "Read the latest policy and prepare the edit again."))
		}
		stored := v.Text
		if !v.Configured {
			stored = ""
		}
		updated := ""
		if input.Content != nil && input.OldText == nil && input.NewText == nil {
			updated = *input.Content
		} else if input.Content == nil && input.OldText != nil && input.NewText != nil {
			old := *input.OldText
			if old == "" || utf8.RuneCountInString(old) > 12000 || utf8.RuneCountInString(*input.NewText) > 12000 || strings.Count(stored, old) != 1 {
				return assistant.AgentToolResult{}, access.ErrInvalid
			}
			updated = strings.Replace(stored, old, *input.NewText, 1)
		} else {
			return assistant.AgentToolResult{}, access.ErrInvalid
		}
		if !utf8.ValidString(updated) || strings.TrimSpace(updated) == "" || utf8.RuneCountInString(updated) > 12000 || strings.ContainsRune(updated, 0) {
			return assistant.AgentToolResult{}, access.ErrInvalid
		}
		if updated == stored {
			return finish(map[string]any{"ok": true, "status": "unchanged", "applied": false, "document": input.Document})
		}
		if access.Missing(s.dependencies.PolicyChanges) {
			return assistant.AgentToolResult{}, access.ErrUnavailable
		}
		raw, e := s.dependencies.PolicyChanges.PrepareSitePolicyChange(ctx, in.Credential, in.Call.OperationID, PolicyChange{input.Document, input.Language, v.Revision, updated})
		if e != nil {
			return assistant.AgentToolResult{}, e
		}
		var value map[string]any
		if decode(raw, &value) != nil || value["applied"] != false || value["requires_confirmation"] != true {
			return assistant.AgentToolResult{}, access.ErrUnavailable
		}
		return result(in.Call.OperationID, assistant.AgentToolPending, value, "service:"+in.Call.OperationID)
	}
	return assistant.AgentToolResult{}, access.ErrNotFound
}

// SimpleFold follows Unicode equivalence cycles without changing rune counts,
// so source offsets remain valid for Kelvin signs, sigma variants and long s.
func policyRuneEqual(a, b rune) bool {
	if a == b {
		return true
	}
	for next := unicode.SimpleFold(a); next != a; next = unicode.SimpleFold(next) {
		if next == b {
			return true
		}
	}
	return false
}

// PublishedLegalReader adapts the exact immutable snapshot mounted for public
// legal pages. No request opens a file or fetches an external policy URL.
type PublishedLegalReader struct{ Handler *legal.Handler }

func (r PublishedLegalReader) PublishedPolicy(ctx context.Context, credential, document, language string) (PublishedPolicy, error) {
	if err := ctx.Err(); err != nil {
		return PublishedPolicy{}, err
	}
	if r.Handler == nil || !policyIdentity(policyInput{Document: document, Language: language}, false) {
		return PublishedPolicy{}, access.ErrInvalid
	}
	v, err := r.Handler.Snapshot(strings.ReplaceAll(document, "_", "-"), language)
	if err != nil {
		return PublishedPolicy{}, access.ErrUnavailable
	}
	source := v.SourceLanguage
	if strings.EqualFold(source, "zh-cn") {
		source = "zh-CN"
	}
	if source == "" && !v.SourceConfigured {
		source = language
	}
	// Bind the fallback choice and requested variant to the displayed revision,
	// including an unpublished/empty snapshot. This does not authorize editing.
	raw, _ := json.Marshal([]any{document, language, source, v.Text, v.Revision, v.RequestedConfigured})
	return PublishedPolicy{Document: document, Language: language, SourceLanguage: source, Text: v.Text, Revision: digest(raw), Configured: v.RequestedConfigured}, nil
}

func policySourceLanguage(value string) bool {
	if value == "" || len(value) > 35 {
		return false
	}
	for _, r := range value {
		if !(r >= 'a' && r <= 'z' || r >= 'A' && r <= 'Z' || r >= '0' && r <= '9' || r == '-') {
			return false
		}
	}
	return true
}
