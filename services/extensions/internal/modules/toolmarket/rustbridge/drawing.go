// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package rustbridge

import (
	"bytes"
	"context"
	"encoding/json"
	"io"
	"net/http"
	"net/url"
	"strings"
	"time"

	market "github.com/TokenNotIncluded/api.lmm.best/extensions/internal/modules/toolmarket"
)

// DrawingAdapter has its own model socket and host token; it holds no supplier
// credential or price and cannot confirm an image operation for the user.
type DrawingAdapter struct {
	bridge *Adapter
	origin string
}

func NewDrawing(socket string, token []byte, coreOrigin string, localHTTP bool) (*DrawingAdapter, error) {
	u, e := url.Parse(coreOrigin)
	if e != nil || u.Host == "" || u.User != nil || u.RawQuery != "" || u.Fragment != "" || u.Path != "" {
		return nil, market.ErrInvalid
	}
	if u.Scheme != "https" && !(localHTTP && u.Scheme == "http" && (u.Hostname() == "127.0.0.1" || u.Hostname() == "::1")) {
		return nil, market.ErrInvalid
	}
	a, e := New(socket, token)
	if e != nil {
		return nil, e
	}
	a.client.Timeout = 125 * time.Second
	if transport, ok := a.client.Transport.(*http.Transport); ok {
		transport.ResponseHeaderTimeout = 120 * time.Second
	}
	return &DrawingAdapter{a, coreOrigin}, nil
}
func (d *DrawingAdapter) Close() {
	if d != nil && d.bridge != nil {
		d.bridge.Close()
	}
}

type imagePreview struct {
	OperationID        string          `json:"operation_id"`
	PreviewDigest      string          `json:"preview_digest"`
	Input              json.RawMessage `json:"input"`
	ReservationCredits int64           `json:"reservation_credits"`
	ExpiresAt          int64           `json:"expires_at"`
	State              string          `json:"state"`
	ConfirmationPath   string          `json:"confirmation_path"`
}
type imageReceipt struct {
	OperationID        string          `json:"operation_id"`
	State              string          `json:"state"`
	PreviewDigest      string          `json:"preview_digest"`
	ReservationCredits int64           `json:"reservation_credits"`
	ChargedCredits     *int64          `json:"charged_credits"`
	Output             json.RawMessage `json:"output"`
}

func (d *DrawingAdapter) requestImage(ctx context.Context, session, action string, in, out any) error {
	if d == nil || d.bridge == nil {
		return market.ErrUnavailable
	}
	raw, e := json.Marshal(in)
	if e != nil || len(raw) > 80<<10 {
		return market.ErrInvalid
	}
	req, e := http.NewRequestWithContext(ctx, http.MethodPost, "http://core-images/internal/models/images/v1/"+action, bytes.NewReader(raw))
	if e != nil {
		return market.ErrInvalid
	}
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Authorization", "Bearer "+d.bridge.token)
	req.Header.Set("X-Lmm-Protocol", "2")
	req.Header.Set("X-Lmm-User-Credential", session)
	res, e := d.bridge.client.Do(req)
	if e != nil {
		return market.ErrPending
	}
	defer res.Body.Close()
	const bound = 32<<20 + 64<<10
	body, e := io.ReadAll(io.LimitReader(res.Body, bound+1))
	if e != nil || len(body) > bound {
		return market.ErrPending
	}
	if res.StatusCode != 200 {
		switch res.StatusCode {
		case 400:
			return market.ErrInvalid
		case 401, 403:
			return market.ErrDenied
		case 404:
			return market.ErrMissing
		case 409:
			return market.ErrConflict
		default:
			return market.ErrPending
		}
	}
	dec := json.NewDecoder(bytes.NewReader(body))
	dec.DisallowUnknownFields()
	if dec.Decode(out) != nil {
		return market.ErrPending
	}
	var extra any
	if dec.Decode(&extra) != io.EOF {
		return market.ErrPending
	}
	return nil
}
func imageEnvelope(value any) json.RawMessage {
	textValue := value
	if fields, ok := value.(map[string]any); ok {
		if _, present := fields["output"]; present {
			metadata := make(map[string]any, len(fields))
			for k, v := range fields {
				if k != "output" {
					metadata[k] = v
				}
			}
			textValue = metadata
		}
	}
	text, _ := json.Marshal(textValue)
	result, _ := json.Marshal(map[string]any{"content": []any{map[string]any{"type": "text", "text": string(text)}}, "structuredContent": value, "isError": false})
	return result
}
func imageResult(call string, r imageReceipt) (market.BuiltinResult, error) {
	out := market.BuiltinResult{CallID: call, DownstreamOperationID: r.OperationID, ExecutionStatus: r.State, SettlementStatus: "pending"}
	if r.OperationID == "" || r.PreviewDigest == "" || r.ReservationCredits < 0 {
		return out, market.ErrPending
	}
	if r.State == "completed" {
		if r.ChargedCredits == nil || *r.ChargedCredits < 0 || len(r.Output) == 0 || string(r.Output) == "null" {
			return out, market.ErrPending
		}
		out.SettlementStatus = "settled"
		out.ExecutionStatus = "completed"
	}
	out.Continuation, _ = json.Marshal(map[string]string{"operation_id": r.OperationID})
	out.ResumeReady = r.State == "confirmed"
	out.AwaitingConfirmation = r.State == "previewed"
	out.Result = imageEnvelope(map[string]any{"operation_id": r.OperationID, "state": r.State, "preview_digest": r.PreviewDigest, "reservation_credits": r.ReservationCredits, "charged_credits": r.ChargedCredits, "output": r.Output, "billing_pending": out.SettlementStatus != "settled"})
	return out, nil
}
func (d *DrawingAdapter) InvokeDrawing(ctx context.Context, session string, in market.BuiltinInvocation, _ string) (market.BuiltinResult, error) {
	out := market.BuiltinResult{CallID: in.Context.CallID}
	if in.ToolName == "drawing.list_capabilities" {
		var models []json.RawMessage
		e := d.requestImage(ctx, session, "capabilities", map[string]any{"context": in.Context}, &models)
		if e != nil {
			return out, e
		}
		out.Result = imageEnvelope(map[string]any{"models": models})
		out.ExecutionStatus = "completed"
		out.SettlementStatus = "settled"
		return out, nil
	}
	var p imagePreview
	e := d.requestImage(ctx, session, "preview", map[string]any{"context": in.Context, "input": in.Arguments}, &p)
	if e != nil {
		return out, e
	}
	if p.OperationID == "" || p.PreviewDigest == "" || p.ReservationCredits < 0 || p.ConfirmationPath != "/core/v1/images/confirm/"+p.OperationID || strings.ContainsAny(p.OperationID, "/?#\r\n") {
		return out, market.ErrPending
	}
	out.DownstreamOperationID = p.OperationID
	out.Continuation, _ = json.Marshal(map[string]string{"operation_id": p.OperationID})
	out.AwaitingConfirmation = p.State == "previewed"
	out.ExecutionStatus = p.State
	out.SettlementStatus = "pending"
	out.Result = imageEnvelope(map[string]any{"preview": p, "confirmation_url": d.origin + p.ConfirmationPath, "billing_pending": true})
	return out, nil
}
func (d *DrawingAdapter) QueryDrawing(ctx context.Context, session string, in market.BuiltinInvocation, id string) (market.BuiltinResult, error) {
	var r imageReceipt
	request := map[string]any{"context": in.Context}
	if id != "" {
		request["operation_id"] = id
	}
	e := d.requestImage(ctx, session, "query", request, &r)
	if e != nil {
		return market.BuiltinResult{CallID: in.Context.CallID}, e
	}
	if id != "" && r.OperationID != id {
		return market.BuiltinResult{}, market.ErrConflict
	}
	return imageResult(in.Context.CallID, r)
}
func (d *DrawingAdapter) ResumeDrawing(ctx context.Context, session string, in market.BuiltinInvocation, id string) (market.BuiltinResult, error) {
	var r imageReceipt
	e := d.requestImage(ctx, session, "execute", map[string]any{"operation_id": id, "context": in.Context}, &r)
	if e != nil {
		return market.BuiltinResult{CallID: in.Context.CallID}, e
	}
	if id != "" && r.OperationID != id {
		return market.BuiltinResult{}, market.ErrConflict
	}
	return imageResult(in.Context.CallID, r)
}
