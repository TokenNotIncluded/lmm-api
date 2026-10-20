// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package toolmarket

import (
	"context"
	"encoding/json"
	"errors"
	"sort"
	"strconv"
)

type LegacyWebExecutionResponse struct {
	Call                 map[string]any  `json:"call"`
	Result               json.RawMessage `json:"result,omitempty"`
	ResultExpiresAt      int64           `json:"result_expires_at,omitempty"`
	ResultExpired        bool            `json:"result_expired"`
	ErrorCode            string          `json:"error_code,omitempty"`
	RequestState         string          `json:"request_state,omitempty"`
	AwaitingConfirmation bool            `json:"awaiting_confirmation"`
}

func (m *Module) legacyWebExecution(r LegacyExecution, pending bool) (LegacyWebExecutionResponse, error) {
	if r.ID == "" {
		return LegacyWebExecutionResponse{}, ErrMissing
	}
	user, err := strconv.ParseInt(r.Owner.User, 10, 64)
	if err != nil || user <= 0 {
		return LegacyWebExecutionResponse{}, ErrDenied
	}
	call := map[string]any{"id": r.ID, "user_id": user, "client_id": r.Input.ClientID, "tool_id": r.Input.ToolID, "version_id": r.Input.VersionID, "grant_id": r.Input.GrantID, "created_at": r.CreatedAt, "resolve_by": r.ResolveBy}
	execution := "reserved"
	switch r.State {
	case "running", "dispatching":
		execution = "running"
	case "unknown":
		execution = "unknown"
	case "awaiting_confirmation":
		execution = "awaiting_confirmation"
	case "done":
		execution = "failed"
		if r.Success {
			execution = "succeeded"
		}
	case "not_reserved":
		execution = "cancelled"
	}
	if r.ResultDigest != "" {
		execution = "failed"
		if r.Success {
			execution = "succeeded"
		}
	}
	settlement := "reserved"
	if r.Core.State == "settled" || r.Core.State == "released" {
		settlement = r.Core.State
	}
	if r.Core.State == "running" && execution == "reserved" {
		execution = "running"
	}
	if r.Core.State == "unknown" && r.ResultDigest == "" && r.State != "awaiting_confirmation" {
		execution = "unknown"
	}
	if r.Builtin != nil && r.Builtin.ResumeClaimed && r.ResultDigest == "" {
		execution = "unknown"
	}
	call["execution_status"], call["settlement_status"] = execution, settlement
	if r.Core.ID != "" {
		price := r.Core.Held
		if r.Core.State == "settled" {
			price = r.Core.Charged
		}
		call["price_quota"], call["fee_quota"] = price, r.Core.Fee
	}
	if r.Usage != nil {
		call["usage_source"], call["usage_quantities"] = r.Usage.Source, r.Usage.Quantities
		call["input_tokens"] = r.Usage.Quantities["input_tokens"]
	}
	if err := m.store.view(func(d database) error {
		if d.Web != nil {
			if version, ok := d.Web.Versions[r.Input.VersionID]; ok {
				call["service_id"] = version.Version.ServiceID
				for _, tool := range version.Tools {
					if tool.ToolID == r.Input.ToolID {
						call["billing_mode"], call["billing_rules"] = tool.BillingMode, tool.BillingRules
						call["input_token_price_quota"], call["max_input_tokens"] = tool.InputTokenPriceQuota, tool.MaxInputTokens
						break
					}
				}
			}
		}
		return nil
	}); err != nil {
		return LegacyWebExecutionResponse{}, err
	}
	result := append(json.RawMessage(nil), r.Result...)
	if len(result) == 0 && r.Builtin != nil {
		result = LegacyBuiltinPublicPreview(r)
	}
	expired := r.ResultExpires > 0 && m.now().Unix() >= r.ResultExpires
	if expired {
		result = nil
	}
	out := LegacyWebExecutionResponse{Call: call, Result: result, ResultExpiresAt: r.ResultExpires, ResultExpired: expired && r.Success}
	if r.Builtin != nil {
		out.RequestState = LegacyMCPRequestState(r)
		out.AwaitingConfirmation = execution == "awaiting_confirmation"
	}
	if pending {
		out.ErrorCode = "TOOL_MARKET_EXECUTION_PENDING"
	}
	return out, nil
}
func (m *Module) legacyWebInvoke(ctx context.Context, credential, account string, in WebRequest) (any, error) {
	if port, ok := m.authority.(CoreLegacyCredits); !ok || missingInterface(port) {
		return nil, ErrFunds
	}
	value, err := payload[struct {
		ToolID         string          `json:"tool_id"`
		VersionID      string          `json:"version_id"`
		GrantID        string          `json:"grant_id"`
		RequestID      string          `json:"request_id"`
		Arguments      json.RawMessage `json:"arguments"`
		RequestState   string          `json:"request_state"`
		InputResponses json.RawMessage `json:"input_responses"`
	}](in.Body, "tool_id", "version_id", "grant_id", "request_id", "arguments")
	if err != nil {
		return nil, err
	}
	record, err := m.LegacyExecuteMCP(WithBuiltinPeerIP(ctx, in.ActualPeerIP), credential, account, LegacyExecuteInput{ClientID: "web-market", RequestKey: value.RequestID, ToolID: value.ToolID, VersionID: value.VersionID, GrantID: value.GrantID, Arguments: value.Arguments}, value.RequestState, value.InputResponses)
	if err != nil && !errors.Is(err, ErrPending) {
		return nil, err
	}
	response, projectionErr := m.legacyWebExecution(record, errors.Is(err, ErrPending))
	if projectionErr != nil {
		return nil, projectionErr
	}
	return response, err
}
func (m *Module) legacyWebCallResult(ctx context.Context, credential, account, id string) (any, error) {
	record, err := m.LegacyInspectExecution(ctx, credential, account, id)
	if err != nil && !errors.Is(err, ErrPending) {
		return nil, err
	}
	response, projectionErr := m.legacyWebExecution(record, errors.Is(err, ErrPending))
	if projectionErr != nil {
		return nil, projectionErr
	}
	return response, err
}

func (m *Module) legacyWebExecutionLookup(ctx context.Context, credential, account, requestID string) (any, error) {
	record, err := m.LegacyGetExecutionByRequestKey(ctx, credential, account, "web-market", requestID)
	if err != nil && !errors.Is(err, ErrPending) {
		return nil, err
	}
	response, projectionErr := m.legacyWebExecution(record, errors.Is(err, ErrPending))
	if projectionErr != nil {
		return nil, projectionErr
	}
	return response, err
}

func (m *Module) legacyWebResume(ctx context.Context, credential, account string, in WebRequest) (any, error) {
	value, err := payload[struct {
		RequestState   string          `json:"request_state"`
		InputResponses json.RawMessage `json:"input_responses"`
	}](in.Body, "request_state")
	if err != nil {
		return nil, err
	}
	record, err := m.LegacyResumeWebExecution(WithBuiltinPeerIP(ctx, in.ActualPeerIP), credential, account, in.ID, value.RequestState, value.InputResponses)
	if err != nil && !errors.Is(err, ErrPending) {
		return nil, err
	}
	response, projectionErr := m.legacyWebExecution(record, errors.Is(err, ErrPending))
	if projectionErr != nil {
		return nil, projectionErr
	}
	return response, err
}
func (m *Module) legacyWebCalls(ctx context.Context, credential, account string, user int64, offset, limit int) (any, error) {
	if port, ok := m.authority.(CoreLegacyCredits); !ok || missingInterface(port) {
		return nil, ErrUnavailable
	}
	if !validWebPage(offset, limit) {
		return nil, ErrInvalid
	}
	var records []LegacyExecution
	if err := m.store.view(func(d database) error {
		for _, r := range d.LegacyExecutions {
			if r.Owner.User == strconv.FormatInt(user, 10) && r.Owner.Account == account {
				records = append(records, r)
			}
		}
		return nil
	}); err != nil {
		return nil, err
	}
	sort.Slice(records, func(i, j int) bool {
		if records[i].CreatedAt != records[j].CreatedAt {
			return records[i].CreatedAt > records[j].CreatedAt
		}
		return records[i].ID < records[j].ID
	})
	records = webSlice(records, offset, limit)
	rows := make([]map[string]any, 0, len(records))
	for _, r := range records {
		latest, err := m.LegacyInspectExecution(ctx, credential, account, r.ID)
		if err != nil && !errors.Is(err, ErrPending) {
			return nil, err
		}
		response, err := m.legacyWebExecution(latest, false)
		if err != nil {
			return nil, err
		}
		rows = append(rows, response.Call)
	}
	return rows, nil
}
