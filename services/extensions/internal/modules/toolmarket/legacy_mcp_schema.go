// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package toolmarket

import (
	"bytes"
	"encoding/json"
	"sort"
	"unicode/utf8"
)

type legacyMCPAction struct{ required, optional []string }

var legacyMCPActions = map[string]legacyMCPAction{
	"search":  {optional: []string{"query", "offset", "limit"}},
	"details": {required: []string{"service_id"}}, "status": {},
	"load": {required: []string{"tool_id", "version_id"}}, "unload": {required: []string{"tool_id", "version_id"}},
	"authorize":         {required: []string{"tool_id", "version_id", "max_price_quota", "max_total_quota", "max_calls", "expires_at"}},
	"set_tool_budget":   {required: []string{"tool_id", "version_id", "grant_id", "limit_quota"}},
	"set_client_budget": {required: []string{"limit_quota"}},
	"usage":             {optional: []string{"offset", "limit"}}, "calls": {optional: []string{"offset", "limit"}},
	"call_status": {required: []string{"call_id"}},
	"invoke":      {required: []string{"tool_id", "version_id", "request_id", "arguments"}},
}

type legacyMCPInput struct {
	Action        string          `json:"action"`
	Query         string          `json:"query"`
	ServiceID     string          `json:"service_id"`
	ToolID        string          `json:"tool_id"`
	VersionID     string          `json:"version_id"`
	GrantID       string          `json:"grant_id"`
	CallID        string          `json:"call_id"`
	RequestID     string          `json:"request_id"`
	Arguments     json.RawMessage `json:"arguments"`
	Offset        int             `json:"offset"`
	Limit         int             `json:"limit"`
	MaxPriceQuota int64           `json:"max_price_quota"`
	MaxTotalQuota int64           `json:"max_total_quota"`
	MaxCalls      int64           `json:"max_calls"`
	ExpiresAt     int64           `json:"expires_at"`
	LimitQuota    int64           `json:"limit_quota"`
}

func legacyMCPDefaultLimit(action string) int {
	if action == "search" {
		return 5
	}
	return 20
}
func legacyDecodeMCPInput(raw []byte, now int64) (legacyMCPInput, error) {
	var in legacyMCPInput
	if len(raw) == 0 || len(raw) > 256<<10 || decodeStrict(raw, &in) != nil {
		return in, ErrInvalid
	}
	var fields map[string]json.RawMessage
	if json.Unmarshal(raw, &fields) != nil || fields == nil {
		return in, ErrInvalid
	}
	spec, ok := legacyMCPActions[in.Action]
	if !ok || in.Action != "invoke" && len(raw) > 8192 {
		return in, ErrInvalid
	}
	permitted := map[string]bool{"action": true}
	for _, k := range spec.required {
		if _, ok := fields[k]; !ok {
			return in, ErrInvalid
		}
		permitted[k] = true
	}
	for _, k := range spec.optional {
		permitted[k] = true
	}
	for k, v := range fields {
		if !permitted[k] || bytes.Equal(bytes.TrimSpace(v), []byte("null")) {
			return in, ErrInvalid
		}
	}
	id := func(v string) bool { return len(v) > 0 && len(v) <= 128 }
	switch in.Action {
	case "details":
		if !id(in.ServiceID) {
			return in, ErrInvalid
		}
	case "load", "unload", "authorize", "set_tool_budget", "invoke":
		if !id(in.ToolID) || !id(in.VersionID) {
			return in, ErrInvalid
		}
	case "call_status":
		if !id(in.CallID) {
			return in, ErrInvalid
		}
	}
	if in.Action == "invoke" {
		args := bytes.TrimSpace(in.Arguments)
		if !id(in.RequestID) || len(args) == 0 || args[0] != '{' || len(in.Arguments) > 128<<10 {
			return in, ErrInvalid
		}
	}
	if in.Action == "authorize" && (!legacyQuotaValid(in.MaxPriceQuota) || !legacyQuotaValid(in.MaxTotalQuota) || in.MaxCalls < 1 || in.MaxCalls > 1000000 || in.ExpiresAt <= now) {
		return in, ErrInvalid
	}
	if in.Action == "set_client_budget" || in.Action == "set_tool_budget" {
		if !legacyQuotaValid(in.LimitQuota) || in.Action == "set_tool_budget" && !id(in.GrantID) {
			return in, ErrInvalid
		}
	}
	if in.Action == "search" || in.Action == "usage" || in.Action == "calls" {
		if _, ok := fields["limit"]; !ok {
			in.Limit = legacyMCPDefaultLimit(in.Action)
		}
		if in.Offset < 0 || in.Offset > 10000 || in.Limit < 1 || in.Limit > 100 {
			return in, ErrInvalid
		}
	}
	if in.Action == "search" && (!utf8.ValidString(in.Query) || utf8.RuneCountInString(in.Query) > 120) {
		return in, ErrInvalid
	}
	return in, nil
}
func legacyMCPSchema(properties map[string]any, required ...string) map[string]any {
	out := map[string]any{"type": "object", "properties": properties, "additionalProperties": false}
	if len(required) > 0 {
		out["required"] = required
	}
	return out
}
func legacyMCPInputSchema() map[string]any {
	actions := make([]string, 0, len(legacyMCPActions))
	for a := range legacyMCPActions {
		actions = append(actions, a)
	}
	sort.Strings(actions)
	quota := map[string]any{"type": "integer", "minimum": 0, "maximum": int64(9007199254740991)}
	props := map[string]any{"action": map[string]any{"type": "string", "enum": actions}, "query": map[string]any{"type": "string", "maxLength": 120}, "arguments": map[string]any{"type": "object", "additionalProperties": true}, "offset": map[string]any{"type": "integer", "minimum": 0, "maximum": 10000, "default": 0}, "limit": map[string]any{"type": "integer", "minimum": 1, "maximum": 100}, "max_price_quota": quota, "max_total_quota": quota, "limit_quota": quota, "max_calls": map[string]any{"type": "integer", "minimum": 1, "maximum": 1000000}, "expires_at": map[string]any{"type": "integer", "minimum": 1}}
	for _, k := range []string{"service_id", "tool_id", "version_id", "grant_id", "call_id", "request_id"} {
		props[k] = map[string]any{"type": "string", "minLength": 1, "maxLength": 128}
	}
	out := legacyMCPSchema(props, "action")
	branches := make([]any, 0, len(actions))
	for _, a := range actions {
		spec := legacyMCPActions[a]
		p := map[string]any{"action": map[string]any{"const": a}}
		for _, k := range spec.required {
			p[k] = map[string]any{}
		}
		for _, k := range spec.optional {
			p[k] = map[string]any{}
			if k == "limit" {
				p[k] = map[string]any{"default": legacyMCPDefaultLimit(a)}
			}
		}
		b := legacyMCPSchema(p, append([]string{"action"}, spec.required...)...)
		b["title"] = a
		branches = append(branches, b)
	}
	out["oneOf"] = branches
	return out
}
