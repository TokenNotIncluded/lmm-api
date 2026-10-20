// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
// Adapted from frozen main 3948a0a service/tool_market_provider{,_recovery}.go.
package toolmarket

import (
	"context"
	"encoding/json"
	"net/http"
	"strings"
	"time"
)

func legacyValidProviderRunID(id string) bool {
	if len(id) < 1 || len(id) > 64 {
		return false
	}
	for _, c := range id {
		if !(c >= 'a' && c <= 'z' || c >= 'A' && c <= 'Z' || c >= '0' && c <= '9' || c == '_' || c == '-') {
			return false
		}
	}
	return true
}
func legacyProviderBusinessResult(pricing LegacyProviderPricing, quote LegacyProviderQuote, raw json.RawMessage) (pending, valid bool, runID string) {
	var exact map[string]any
	if strictLegacyJSON(raw, &exact, false) != nil || exact == nil {
		return false, false, ""
	}
	if failed, _ := exact["isError"].(bool); failed {
		return false, false, ""
	}
	data, e := legacyProviderPayload(exact)
	if e != nil {
		return false, false, ""
	}
	if pricing.Provider != "monid" {
		if flag, ok := data["success"].(bool); ok && !flag {
			return false, false, ""
		}
		if err, ok := data["error"]; ok && err != nil && err != "" {
			return false, false, ""
		}
		return false, true, ""
	}
	state, _ := data["status"].(string)
	switch strings.ToUpper(state) {
	case "READY", "RUNNING", "PENDING", "QUEUED", "PROCESSING":
		if data["provider"] != quote.TargetProvider || data["endpoint"] != quote.TargetEndpoint {
			return true, false, ""
		}
		if state, ok := exact["requestState"].(string); ok && state != "" {
			return true, false, ""
		}
		if requests, ok := exact["inputRequests"].([]any); ok && len(requests) > 0 {
			return true, false, ""
		}
		id, _ := data["runId"].(string)
		if !legacyValidProviderRunID(id) {
			return true, false, ""
		}
		return true, false, id
	case "COMPLETED":
		if data["provider"] != quote.TargetProvider || data["endpoint"] != quote.TargetEndpoint {
			return false, false, ""
		}
		response, _ := data["providerResponse"].(map[string]any)
		code, ok := response["httpStatus"].(json.Number)
		if !ok {
			return false, false, ""
		}
		n, e := code.Int64()
		return false, e == nil && n >= 200 && n < 300, ""
	default:
		return false, false, ""
	}
}
func legacyMonidPublicResult(raw json.RawMessage) json.RawMessage {
	var exact map[string]any
	if strictLegacyJSON(raw, &exact, false) != nil {
		return legacyFailureResult("The provider returned an invalid result.")
	}
	data, e := legacyProviderPayload(exact)
	if e != nil {
		return legacyFailureResult("The provider returned an invalid result.")
	}
	safe := map[string]any{}
	for _, key := range []string{"runId", "provider", "providerName", "endpoint", "status", "output", "price", "billing", "resultCount", "cost", "createdAt", "startedAt", "completedAt"} {
		if v, ok := data[key]; ok {
			safe[key] = v
		}
	}
	if response, ok := data["providerResponse"].(map[string]any); ok {
		filtered := map[string]any{}
		for _, key := range []string{"httpStatus", "error"} {
			if v, ok := response[key]; ok {
				filtered[key] = v
			}
		}
		safe["providerResponse"] = filtered
	}
	text, _ := json.Marshal(safe)
	out, _ := json.Marshal(map[string]any{"structuredContent": safe, "content": []any{map[string]any{"type": "text", "text": string(text)}}})
	return out
}
func (m *Module) legacyBindProviderRun(r LegacyExecution, runID string) (LegacyExecution, error) {
	if r.ProviderQuote == nil || r.ProviderQuote.Provider != "monid" || !legacyValidProviderRunID(runID) {
		return r, ErrInvalid
	}
	e := m.store.update(func(d *database) error {
		old, ok := d.LegacyExecutions[r.ID]
		if !ok || old.RequestHash != r.RequestHash || old.Owner != r.Owner {
			return ErrConflict
		}
		if old.ProviderRunID != "" && old.ProviderRunID != runID {
			return ErrConflict
		}
		old.ProviderRunID = runID
		old.ProviderNextPollAt = m.now().Unix() + 2
		old.State = "unknown"
		d.LegacyExecutions[r.ID] = old
		r = old
		return nil
	})
	return r, e
}

// LegacyRecoverProviderCalls only polls a durably bound Monid run. No tool
// result can select another URL, merchant history query or repeat monid_run.
func (m *Module) LegacyRecoverProviderCalls(ctx context.Context, limit int) (int, error) {
	if limit < 1 || limit > 1000 {
		return 0, ErrInvalid
	}
	port, ok := m.authority.(CoreLegacyCredits)
	if !ok || missingInterface(port) {
		return 0, ErrUnavailable
	}
	records := []LegacyExecution{}
	e := m.store.update(func(d *database) error {
		now := m.now().Unix()
		for id, r := range d.LegacyExecutions {
			if r.ProviderRunID == "" || r.ProviderQuote == nil || r.ProviderQuote.Provider != "monid" || r.State == "done" || r.ResolveBy <= now || r.ResultDigest != "" || r.ProviderNextPollAt > now || r.ProviderPollLeaseUntil > now {
				continue
			}
			r.ProviderPollLeaseUntil = now + 10
			r.ProviderNextPollAt = now + 2
			d.LegacyExecutions[id] = r
			records = append(records, r)
			if len(records) >= limit {
				break
			}
		}
		return nil
	})
	if e != nil {
		return 0, e
	}
	count := 0
	for _, r := range records {
		pollctx, cancel := context.WithTimeout(ctx, 4*time.Second)
		e = m.legacyPollProviderRun(pollctx, port, r)
		cancel()
		if e != nil && e != ErrPending {
			return count, e
		}
		count++
	}
	return count, nil
}
func (m *Module) legacyPollProviderRun(ctx context.Context, port CoreLegacyCredits, r LegacyExecution) error {
	if r.ProviderQuote == nil || r.ProviderQuote.Provider != "monid" || !legacyValidProviderRunID(r.ProviderRunID) {
		return ErrInvalid
	}
	preset, _ := legacyProviderPreset("monid")
	var credential webCredential
	var tool WebTool
	e := m.store.view(func(d database) error {
		v, ok := d.Web.Versions[r.Input.VersionID]
		if !ok || v.Version.Endpoint != preset.Endpoint {
			return ErrConflict
		}
		var err error
		credential, err = webResolveCredential(d, r.ProviderOwnerID, r.MeteringBinding.ServiceID, r.Input.VersionID, preset.Endpoint)
		if err != nil {
			return err
		}
		for _, t := range v.Tools {
			if t.ToolID == r.Input.ToolID {
				tool = t
				break
			}
		}
		if credential.Mode != "bearer" || tool.ProviderPricing == nil || tool.ProviderPricing.Validate(preset.Endpoint, tool.Name) != nil {
			return ErrDenied
		}
		return nil
	})
	if e != nil {
		return e
	}
	headers := http.Header{"Accept": []string{"application/json"}, "Authorization": []string{"Bearer " + credential.Secret}}
	response, e := m.net.request(ctx, http.MethodGet, "https://api.monid.ai/v1/runs/"+r.ProviderRunID, nil, headers)
	if e != nil {
		return e
	}
	defer response.Body.Close()
	if response.StatusCode != http.StatusOK {
		return ErrUpstream
	}
	raw, e := readBodyLimit(response, 2<<20)
	if e != nil {
		return e
	}
	var data map[string]any
	if strictLegacyJSON(raw, &data, false) != nil || data == nil || legacyResultContains(data, []string{credential.Secret}) || data["runId"] != r.ProviderRunID || data["provider"] != r.ProviderQuote.TargetProvider || data["endpoint"] != r.ProviderQuote.TargetEndpoint {
		return ErrUpstream
	}
	text, _ := json.Marshal(data)
	envelope, _ := json.Marshal(map[string]any{"structuredContent": data, "content": []any{map[string]any{"type": "text", "text": string(text)}}})
	_, e = m.legacyFinishRemote(ctx, port, r, tool, envelope, "", credential.Secret)
	return e
}
