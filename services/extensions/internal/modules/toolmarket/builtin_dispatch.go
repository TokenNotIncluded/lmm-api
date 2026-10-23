// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package toolmarket

import (
	"context"
	"encoding/json"
	"strconv"
	"strings"
	"sync/atomic"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/bounties"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/wallet"
)

type BuiltinContext struct {
	CallID      string `json:"call_id"`
	DispatchID  string `json:"dispatch_id"`
	ToolID      string `json:"tool_id"`
	VersionID   string `json:"version_id"`
	GrantID     string `json:"grant_id"`
	InputDigest string `json:"input_digest"`
}
type BuiltinAuthorizationRequest struct {
	Context           BuiltinContext  `json:"context"`
	ServiceKey        string          `json:"service_key"`
	ToolName          string          `json:"tool_name"`
	InputSchemaDigest string          `json:"input_schema_digest"`
	Arguments         json.RawMessage `json:"arguments"`
}
type BuiltinAuthorization struct {
	Owner             Principal `json:"owner"`
	CallID            string    `json:"call_id"`
	DispatchID        string    `json:"dispatch_id"`
	ToolID            string    `json:"tool_id"`
	VersionID         string    `json:"version_id"`
	InputDigest       string    `json:"input_digest"`
	InputSchemaDigest string    `json:"input_schema_digest"`
}
type BuiltinGate interface {
	AuthorizeBuiltin(context.Context, string, BuiltinAuthorizationRequest) (BuiltinAuthorization, error)
}
type BuiltinInvocation struct {
	Context      BuiltinContext
	Owner        Principal
	ServiceKey   string
	ToolName     string
	Arguments    json.RawMessage
	Continuation json.RawMessage
	ActualPeerIP string
}
type BuiltinResult struct {
	DownstreamOperationID string          `json:"downstream_operation_id,omitempty"`
	CallID                string          `json:"call_id"`
	Result                json.RawMessage `json:"result"`
	ResumeReady           bool            `json:"resume_ready"`
	AwaitingConfirmation  bool            `json:"awaiting_confirmation"`
	Continuation          json.RawMessage `json:"continuation,omitempty"`
	SettlementStatus      string          `json:"settlement_status"`
	ExecutionStatus       string          `json:"execution_status"`
}

// DrawingPort executes only Core-confirmed operations and returns its durable
// billing state. UNKNOWN must query the same operation, never generate again.
type DrawingPort interface {
	InvokeDrawing(context.Context, string, BuiltinInvocation, string) (BuiltinResult, error)
	QueryDrawing(context.Context, string, BuiltinInvocation, string) (BuiltinResult, error)
	ResumeDrawing(context.Context, string, BuiltinInvocation, string) (BuiltinResult, error)
}
type BuiltinDispatcher struct {
	ready            atomic.Bool
	Gate             BuiltinGate
	Wallet           *wallet.Service
	Bounties         *bounties.Service
	Drawing          DrawingPort
	CorePublicOrigin string
}

func (d *BuiltinDispatcher) InvokeBuiltin(ctx context.Context, session string, in BuiltinInvocation) (BuiltinResult, error) {
	out := BuiltinResult{CallID: in.Context.CallID}
	if d == nil || d.Gate == nil || session == "" || !in.Owner.valid() {
		return out, ErrDenied
	}
	definition, tool, e := compiledBuiltin(in.ServiceKey, in.ToolName)
	if e != nil || in.Context.ToolID != tool.ToolID || in.Context.VersionID != definition.VersionID {
		return out, ErrDenied
	}
	if e = validateArgs(json.RawMessage(tool.InputSchema), in.Arguments); e != nil {
		return out, e
	}
	var args map[string]any
	if e = decodeStrict(in.Arguments, &args); e != nil {
		return out, e
	}
	if in.Context.InputDigest != digest(args) {
		return out, ErrConflict
	}
	canonical, _ := json.Marshal(args)
	auth, e := d.Gate.AuthorizeBuiltin(ctx, session, BuiltinAuthorizationRequest{in.Context, in.ServiceKey, in.ToolName, tool.InputSchemaDigest, canonical})
	if e != nil {
		return out, e
	}
	if auth.Owner != in.Owner || auth.CallID != in.Context.CallID || auth.DispatchID != in.Context.DispatchID || auth.ToolID != in.Context.ToolID || auth.VersionID != in.Context.VersionID || auth.InputDigest != in.Context.InputDigest || auth.InputSchemaDigest != tool.InputSchemaDigest {
		return out, ErrDenied
	}
	key := digest([]string{"builtin/v1", in.Context.CallID, in.Context.ToolID})
	var value any
	switch in.ServiceKey {
	case "wallet":
		if d.Wallet == nil {
			return out, ErrUnavailable
		}
		switch in.ToolName {
		case "wallet.balance":
			var b wallet.Balance
			b, e = d.Wallet.Balance(ctx, session)
			value = map[string]any{"schema_version": 2, "unit": "CREDIT", "credits_per_usd": b.CreditsPerUSD, "available_quota": b.AvailableCredits, "available_credits": b.AvailableCredits, "spendable_credits": b.SpendableCredits, "debt_credits": b.DebtCredits, "reserved_credits": b.ReservedCredits, "affiliate_credits": b.AffiliateCredits}
		case "wallet.topup_link":
			value, e = d.Wallet.TopupLink(ctx, session, int(builtinInteger(args, "amount")))
		case "wallet.transfers.list":
			value, e = d.Wallet.List(ctx, session, builtinInteger(args, "before_id"))
		case "wallet.transfer.create":
			var t wallet.Transfer
			t, e = d.Wallet.Create(ctx, session, wallet.CreateInput{AmountCredits: builtinInteger(args, "quota"), RequestKey: key})
			value = t
			out.DownstreamOperationID = strconv.FormatInt(t.ID, 10)
			out.AwaitingConfirmation = t.State == "draft" || t.State == "awaiting_confirmation"
		case "wallet.transfer.cancel":
			var t wallet.Transfer
			t, e = d.Wallet.Cancel(ctx, session, builtinInteger(args, "transfer_id"))
			value = t
			out.DownstreamOperationID = strconv.FormatInt(t.ID, 10)
			out.AwaitingConfirmation = t.State != "cancelled"
		}
	case "open_source_bounties":
		if d.Bounties == nil {
			return out, ErrUnavailable
		}
		var continuation struct {
			Confirmation string `json:"confirmation"`
			OperationID  string `json:"operation_id"`
		}
		if len(in.Continuation) > 0 {
			if e = decodeStrict(in.Continuation, &continuation); e != nil {
				return out, e
			}
		}
		action := strings.TrimPrefix(in.ToolName, "open_source_bounties.")
		preview := continuation.Confirmation == "" && action != "create_draft" && action != "update_draft"
		invocation := builtinBountyInvocation(in, args, key, continuation.Confirmation, preview)
		invocation.Actor.Token = session
		var result bounties.Result
		result, e = d.Bounties.BuiltinInvoke(ctx, invocation)
		if result.Receipt != nil && result.Receipt.ConfirmationURL != "" {
			if !strings.HasPrefix(result.Receipt.ConfirmationURL, "/core/v1/bounties/confirm/") || d.CorePublicOrigin == "" {
				return out, ErrUnavailable
			}
			result.Receipt.ConfirmationURL = d.CorePublicOrigin + result.Receipt.ConfirmationURL
		}
		value = result
		out.DownstreamOperationID = key
		if result.Receipt != nil && result.Receipt.Confirmation != "" {
			out.AwaitingConfirmation = true
			out.Continuation, _ = json.Marshal(struct {
				Confirmation string `json:"confirmation"`
			}{result.Receipt.Confirmation})
		}
	case "drawing":
		if d.Drawing == nil {
			return out, ErrUnavailable
		}
		return d.Drawing.InvokeDrawing(ctx, session, in, key)
	default:
		return out, ErrInvalid
	}
	if e != nil {
		return out, e
	}
	if out.DownstreamOperationID != "" {
		var private map[string]any
		if len(out.Continuation) > 0 {
			_ = json.Unmarshal(out.Continuation, &private)
		}
		if private == nil {
			private = map[string]any{}
		}
		private["operation_id"] = out.DownstreamOperationID
		out.Continuation, _ = json.Marshal(private)
	}
	if out.AwaitingConfirmation {
		out.ExecutionStatus = "awaiting_confirmation"
		out.SettlementStatus = "pending"
	} else {
		out.ExecutionStatus = "completed"
		out.SettlementStatus = "settled"
	}
	text, _ := json.Marshal(value)
	out.Result, e = json.Marshal(map[string]any{"content": []any{map[string]any{"type": "text", "text": string(text)}}, "structuredContent": value, "isError": false})
	return out, e
}
func builtinInteger(v map[string]any, key string) int64 {
	n, _ := v[key].(json.Number)
	i, _ := n.Int64()
	return i
}
func builtinString(v map[string]any, key string) string { s, _ := v[key].(string); return s }

// QueryBuiltin only observes the original downstream operation. Its continuation
// is private persisted host state, never a user-supplied replacement request.
func (d *BuiltinDispatcher) QueryBuiltin(ctx context.Context, session string, in BuiltinInvocation) (BuiltinResult, error) {
	out := BuiltinResult{CallID: in.Context.CallID, SettlementStatus: "pending"}
	if d == nil || session == "" || !in.Owner.valid() {
		return out, ErrDenied
	}
	definition, tool, e := compiledBuiltin(in.ServiceKey, in.ToolName)
	if e != nil || definition.VersionID != in.Context.VersionID || tool.ToolID != in.Context.ToolID {
		definition, tool, e = drawingRecoveryInvocation(ctx, in)
		if e != nil {
			return out, ErrDenied
		}
	}
	var arguments map[string]any
	if decodeStrict(in.Arguments, &arguments) != nil || digest(arguments) != in.Context.InputDigest {
		return out, ErrConflict
	}
	var continuation struct {
		OperationID  string `json:"operation_id"`
		Confirmation string `json:"confirmation"`
	}
	if len(in.Continuation) > 0 && decodeStrict(in.Continuation, &continuation) != nil {
		return out, ErrConflict
	}
	out.DownstreamOperationID = continuation.OperationID
	out.Continuation = in.Continuation
	key := digest([]string{"builtin/v1", in.Context.CallID, in.Context.ToolID})
	var value any
	switch in.ServiceKey {
	case "wallet":
		if d.Wallet == nil {
			return out, ErrUnavailable
		}
		id, e := strconv.ParseInt(continuation.OperationID, 10, 64)
		if continuation.OperationID == "" && in.ToolName == "wallet.transfer.cancel" {
			id = builtinInteger(arguments, "transfer_id")
			e = nil
		}
		if continuation.OperationID != "" && (e != nil || id <= 0) {
			return out, ErrConflict
		}
		var t wallet.Transfer
		if in.ToolName == "wallet.transfer.create" && continuation.OperationID == "" {
			t, e = d.Wallet.QueryTransferByRequestKey(ctx, session, key)
			id = t.ID
		} else if in.ToolName == "wallet.transfer.cancel" {
			t, e = d.Wallet.QueryCancellation(ctx, session, id)
		} else {
			t, e = d.Wallet.QueryTransfer(ctx, session, id)
		}
		if e != nil {
			return out, e
		}
		if in.ToolName == "wallet.transfer.create" {
			if t.RequestKey != key || t.AmountCredits != builtinInteger(arguments, "quota") {
				return out, ErrConflict
			}
			out.AwaitingConfirmation = t.State == "awaiting_confirmation"
			if !out.AwaitingConfirmation && t.State != "pending" && t.State != "claimed" && t.State != "cancelled" {
				return out, ErrPending
			}
		} else if in.ToolName == "wallet.transfer.cancel" {
			if id != builtinInteger(arguments, "transfer_id") {
				return out, ErrConflict
			}
			out.AwaitingConfirmation = t.State == "pending"
			if !out.AwaitingConfirmation && t.State != "cancelled" {
				return out, ErrConflict
			}
		} else {
			return out, ErrInvalid
		}
		value = t
		out.DownstreamOperationID = strconv.FormatInt(t.ID, 10)
		out.Continuation, _ = json.Marshal(map[string]string{"operation_id": out.DownstreamOperationID})
		if out.AwaitingConfirmation {
			out.ExecutionStatus = "awaiting_confirmation"
		} else {
			out.ExecutionStatus = "completed"
			out.SettlementStatus = "settled"
		}
	case "open_source_bounties":
		if d.Bounties == nil {
			return out, ErrUnavailable
		}
		if continuation.OperationID != "" && continuation.OperationID != key {
			return out, ErrConflict
		}
		out.DownstreamOperationID = key
		invocation := builtinBountyInvocation(in, arguments, key, continuation.Confirmation, false)
		invocation.Actor.Token = session
		recovered, e := d.Bounties.RecoverOperation(ctx, session, invocation)
		if e != nil {
			return out, e
		}
		if recovered.Receipt == nil {
			return out, ErrPending
		}
		r := *recovered.Receipt
		if r.ConfirmationURL != "" {
			if !strings.HasPrefix(r.ConfirmationURL, "/core/v1/bounties/confirm/") || d.CorePublicOrigin == "" {
				return out, ErrUnavailable
			}
			recovered.Receipt.ConfirmationURL = d.CorePublicOrigin + r.ConfirmationURL
		}
		value = recovered
		out.ExecutionStatus = r.State
		out.AwaitingConfirmation = r.State == "awaiting_confirmation"
		out.ResumeReady = r.State == "confirmed"
		out.Continuation, _ = json.Marshal(map[string]string{"operation_id": key, "confirmation": r.Confirmation})
		switch r.State {
		case "awaiting_confirmation", "confirmed", "confirmation_expired":
		case "unknown", "running", "dispatching", "billing_pending":
			return out, ErrPending
		default:
			out.ExecutionStatus = "completed"
			out.SettlementStatus = "settled"
		}
	case "drawing":
		if d.Drawing == nil {
			return out, ErrUnavailable
		}
		out, e = d.Drawing.QueryDrawing(ctx, session, in, continuation.OperationID)
		if e != nil {
			return out, e
		}
		out.ResumeReady = out.ExecutionStatus == "confirmed"
		return out, nil
	default:
		return out, ErrInvalid
	}
	text, _ := json.Marshal(value)
	out.Result, e = json.Marshal(map[string]any{"content": []any{map[string]any{"type": "text", "text": string(text)}}, "structuredContent": value, "isError": false})
	return out, e
}

// ResumeBuiltin is used only after durable confirmation observation and a host
// CAS claim. An ambiguous execution may only return to QueryBuiltin.
func (d *BuiltinDispatcher) ResumeBuiltin(ctx context.Context, session string, in BuiltinInvocation) (BuiltinResult, error) {
	r, e := d.QueryBuiltin(ctx, session, in)
	if e != nil {
		return r, e
	}
	if !r.ResumeReady {
		return r, ErrPending
	}
	if in.ServiceKey == "drawing" {
		return d.Drawing.ResumeDrawing(ctx, session, in, r.DownstreamOperationID)
	}
	if in.ServiceKey == "open_source_bounties" {
		in.Continuation = r.Continuation
		return d.InvokeBuiltin(ctx, session, in)
	}
	return r, ErrInvalid
}

func builtinBountyInvocation(in BuiltinInvocation, args map[string]any, key, confirmation string, preview bool) bounties.Invocation {
	action := strings.TrimPrefix(in.ToolName, "open_source_bounties.")
	c := bounties.Command{OperationID: key, Action: action, ProjectID: builtinString(args, "project_id"), ChallengeID: builtinString(args, "challenge_id"), DisputeID: builtinString(args, "dispute_id"), Confirmation: confirmation, GithubHandle: builtinString(args, "github_handle"), IssueURL: builtinString(args, "issue_url"), PullRequestURL: builtinString(args, "pull_request_url"), SubmissionNote: builtinString(args, "submission_note"), ReviewNote: builtinString(args, "review_note"), RatingScore: int(builtinInteger(args, "rating_score")), RatingComment: builtinString(args, "rating_comment"), Reason: builtinString(args, "reason"), Statement: builtinString(args, "statement"), Resolution: builtinString(args, "resolution"), ResolveAction: builtinString(args, "action"), Credit: builtinInteger(args, "quota")}
	if action == "rate_owner" {
		c.RatingScore = int(builtinInteger(args, "score"))
		c.RatingComment = builtinString(args, "comment")
	}
	if action == "tip" {
		c.ReviewNote = builtinString(args, "note")
	}
	draft := bounties.Draft{Title: builtinString(args, "title"), Description: builtinString(args, "description"), Rules: builtinString(args, "rules"), RepositoryURL: builtinString(args, "repository_url"), RewardCredit: builtinInteger(args, "reward_quota"), RewardSlots: int(builtinInteger(args, "reward_slots"))}
	return bounties.Invocation{Actor: bounties.Actor{}, Command: c, Draft: draft, Page: int(builtinInteger(args, "page")), PageSize: int(builtinInteger(args, "page_size")), Limit: int(builtinInteger(args, "limit")), Status: builtinString(args, "status"), Preview: preview}
}

// CheckReadiness records only live acknowledgements from all configured Core
// authorities. It cannot confirm a user operation or change any financial state.
func (d *BuiltinDispatcher) CheckReadiness(ctx context.Context) error {
	if d == nil {
		return ErrUnavailable
	}
	d.ready.Store(false)
	gate, ok := d.Gate.(interface{ ReadyBuiltins(context.Context) error })
	if !ok {
		return ErrUnavailable
	}
	if e := gate.ReadyBuiltins(ctx); e != nil {
		return e
	}
	for _, port := range []any{d.Wallet, d.Bounties, d.Drawing} {
		probe, ok := port.(interface{ Ready(context.Context) error })
		if !ok || missingInterface(port) {
			return ErrUnavailable
		}
		if e := probe.Ready(ctx); e != nil {
			return e
		}
	}
	d.ready.Store(true)
	return nil
}
func (d *BuiltinDispatcher) LegacyBuiltinReady() bool { return d != nil && d.ready.Load() }
