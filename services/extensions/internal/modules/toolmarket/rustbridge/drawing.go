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

const imageConfirmationPathPrefix = "/api/user/auth/images/confirm/"

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
	ExpiresAt          int64           `json:"expires_at,omitempty"`
}

func (d *DrawingAdapter) requestImage(ctx context.Context, session, action string, in, out any) error {
	return d.requestImageBound(ctx, session, action, in, out, 80<<10)
}

func (d *DrawingAdapter) requestImageBound(ctx context.Context, session, action string, in, out any, requestLimit int) error {
	if d == nil || d.bridge == nil {
		return market.ErrUnavailable
	}
	raw, e := json.Marshal(in)
	if e != nil || len(raw) > requestLimit {
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
func imageResult(call string, r imageReceipt, origin ...string) (market.BuiltinResult, error) {
	out := market.BuiltinResult{CallID: call, DownstreamOperationID: r.OperationID, ExecutionStatus: r.State, SettlementStatus: "pending"}
	if r.OperationID == "" || len(r.OperationID) > 200 || strings.ContainsAny(r.OperationID, "/?#%\\\r\n") || r.PreviewDigest == "" || r.ReservationCredits < 0 {
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
	document := map[string]any{"operation_id": r.OperationID, "state": r.State, "preview_digest": r.PreviewDigest, "reservation_credits": r.ReservationCredits, "charged_credits": r.ChargedCredits, "output": r.Output, "billing_pending": out.SettlementStatus != "settled"}
	if (out.AwaitingConfirmation || out.ResumeReady) && len(origin) == 1 {
		document["confirmation_url"] = origin[0] + imageConfirmationPathPrefix + r.OperationID
		if r.ExpiresAt > 0 {
			document["preview"] = map[string]any{"operation_id": r.OperationID, "state": r.State, "expires_at": r.ExpiresAt, "reservation_credits": r.ReservationCredits, "preview_digest": r.PreviewDigest}
		}
	}
	out.Result = imageEnvelope(document)
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
	input := map[string]any{"context": in.Context, "input": in.Arguments}
	var e error
	if uploads := market.DrawingUploads(ctx); len(uploads) > 0 {
		if in.ToolName != "drawing.generate" || len(uploads) > market.DrawingUploadCountLimit {
			return out, market.ErrInvalid
		}
		input["uploads"] = uploads
		// Only this initial preview accepts the bounded transient sidecar.
		// Execute/query keep the original small command bound and use Core's
		// frozen image bytes, never a browser replacement upload.
		e = d.requestImageBound(ctx, session, "preview", input, &p, (24<<20)+(64<<10))
	} else {
		e = d.requestImage(ctx, session, "preview", input, &p)
	}
	if e != nil {
		return out, e
	}
	if p.OperationID == "" || len(p.OperationID) > 200 || p.PreviewDigest == "" || p.ReservationCredits < 0 || p.ConfirmationPath != imageConfirmationPathPrefix+p.OperationID || strings.ContainsAny(p.OperationID, "/?#%\\\r\n") {
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
	return imageResult(in.Context.CallID, r, d.origin)
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
	return imageResult(in.Context.CallID, r, d.origin)
}
