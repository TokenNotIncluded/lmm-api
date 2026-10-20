package bounties

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/url"
	"path/filepath"
	"strconv"
	"strings"
	"time"
)

// ErrUnknown means execution may have committed in Core. Callers may retry only
// the identical operation ID/payload; they must never invent another operation.
var ErrUnknown = errors.New("bounty execution outcome unknown")

type CoreClient struct {
	http         *http.Client
	serviceToken string
}

// NewCoreClient only dials the configured Unix socket, never TCP. The service
// token authorizes this module, while each call also requires a real Session.
func NewCoreClient(socket, token string) (*CoreClient, error) {
	if !filepath.IsAbs(socket) || len(socket) > 107 || len(token) < 32 || len(token) > 256 || strings.ContainsAny(token, "\r\n\x00") {
		return nil, ErrInvalid
	}
	dial := &net.Dialer{Timeout: 3 * time.Second}
	transport := &http.Transport{Proxy: nil, DisableKeepAlives: true, ResponseHeaderTimeout: 8 * time.Second, DialContext: func(ctx context.Context, _, _ string) (net.Conn, error) { return dial.DialContext(ctx, "unix", socket) }}
	return &CoreClient{http: &http.Client{Transport: transport, Timeout: 10 * time.Second, CheckRedirect: func(_ *http.Request, _ []*http.Request) error { return http.ErrUseLastResponse }}, serviceToken: token}, nil
}
func (c *CoreClient) call(ctx context.Context, session, action string, payload, out any) error {
	if c == nil || c.http == nil || (session == "" && action != "config" && action != "ready") || len(session) > 4096 || strings.ContainsAny(session, "\r\n\x00") {
		return ErrForbidden
	}
	b, e := json.Marshal(payload)
	if e != nil || len(b) > 24576 {
		return ErrInvalid
	}
	req, e := http.NewRequestWithContext(ctx, http.MethodPost, "http://core/internal/bounties/v1/"+action, bytes.NewReader(b))
	if e != nil {
		return ErrInvalid
	}
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Authorization", "Bearer "+c.serviceToken)
	req.Header.Set("X-Lmm-Protocol", "1")
	req.Header.Set("X-Lmm-User-Credential", session)
	resp, e := c.http.Do(req)
	if e != nil {
		if action == "execute" {
			return ErrUnknown
		}
		return ErrUnavailable
	}
	defer resp.Body.Close()
	switch resp.StatusCode {
	case 200:
	case 400:
		return ErrInvalid
	case 401, 403:
		return ErrForbidden
	case 404:
		return ErrNotFound
	case 409:
		return ErrConflict
	default:
		if action == "execute" {
			return ErrUnknown
		}
		return ErrUnavailable
	}
	limited := io.LimitReader(resp.Body, 65537)
	raw, e := io.ReadAll(limited)
	if e != nil || len(raw) > 65536 {
		if action == "execute" {
			return ErrUnknown
		}
		return ErrUnavailable
	}
	if json.Unmarshal(raw, out) != nil {
		if action == "execute" {
			return ErrUnknown
		}
		return ErrUnavailable
	}
	return nil
}
func (c *CoreClient) Authorize(ctx context.Context, session string) (Actor, error) {
	return c.authorizePath(ctx, session, "authorize")
}
func (c *CoreClient) SessionAuthorize(ctx context.Context, session string) (Actor, error) {
	return c.authorizePath(ctx, session, "session-authorize")
}
func (c *CoreClient) authorizePath(ctx context.Context, session, path string) (Actor, error) {
	var wire struct {
		UserID int64 `json:"user_id"`
		Admin  bool  `json:"admin"`
	}
	if e := c.call(ctx, session, path, struct{}{}, &wire); e != nil {
		return Actor{}, e
	}
	if wire.UserID <= 0 {
		return Actor{}, ErrForbidden
	}
	return Actor{UserID: strconv.FormatInt(wire.UserID, 10), Admin: wire.Admin}, nil
}
func (c *CoreClient) Preview(ctx context.Context, session string, cmd Command) (Receipt, error) {
	var r Receipt
	if e := c.call(ctx, session, "preview", cmd, &r); e != nil {
		return r, e
	}
	if !validID(r.Confirmation) || r.Expires <= 0 {
		return Receipt{}, ErrUnavailable
	}
	r.OperationID = cmd.OperationID
	r.ProjectID = cmd.ProjectID
	r.ChallengeID = cmd.ChallengeID
	r.DisputeID = cmd.DisputeID
	r.State = "awaiting_confirmation"
	r.ConfirmationURL = "/core/v1/bounties/confirm/" + url.PathEscape(r.Confirmation)
	return r, nil
}

// Core uses numeric identity IDs; the extension record boundary uses decimal
// strings. An explicit field overrides the embedded projection JSON tag.
type coreReceipt struct {
	Receipt
	ActorID int64 `json:"actor_user_id"`
}

func (w coreReceipt) projection() Receipt {
	r := w.Receipt
	r.ActorUserID = strconv.FormatInt(w.ActorID, 10)
	if r.Confirmation != "" {
		r.ConfirmationURL = "/core/v1/bounties/confirm/" + url.PathEscape(r.Confirmation)
	}
	return r
}
func (c *CoreClient) Execute(ctx context.Context, session string, cmd Command) (Receipt, error) {
	var wire coreReceipt
	if e := c.call(ctx, session, "execute", cmd, &wire); e != nil {
		return Receipt{}, e
	}
	r := wire.projection()
	if wire.ActorID <= 0 || r.OperationID != cmd.OperationID || r.ProjectID != cmd.ProjectID || r.ChallengeID != cmd.ChallengeID || r.DisputeID != cmd.DisputeID || r.State == "" {
		return Receipt{}, ErrUnknown
	}
	return r, nil
}

// Query only observes Core's existing immutable operation/confirmation records.
// A missing or unknown result never causes a preview or execution retry.
func (c *CoreClient) Query(ctx context.Context, session, operationID string) (Receipt, error) {
	return c.readQuery(ctx, session, operationID, struct {
		OperationID string `json:"operation_id"`
	}{operationID})
}
func (c *CoreClient) QueryForCommand(ctx context.Context, session string, cmd Command) (Receipt, error) {
	r, e := c.readQuery(ctx, session, cmd.OperationID, struct {
		OperationID     string  `json:"operation_id"`
		ExpectedCommand Command `json:"expected_command"`
	}{cmd.OperationID, cmd})
	if e != nil {
		return Receipt{}, e
	}
	if r.ProjectID != cmd.ProjectID || r.ChallengeID != cmd.ChallengeID || r.DisputeID != cmd.DisputeID {
		return Receipt{}, ErrConflict
	}
	return r, nil
}
func (c *CoreClient) readQuery(ctx context.Context, session, operationID string, payload any) (Receipt, error) {
	if !validID(operationID) {
		return Receipt{}, ErrInvalid
	}
	var wire coreReceipt
	if e := c.call(ctx, session, "query", payload, &wire); e != nil {
		return Receipt{}, e
	}
	r := wire.projection()
	if wire.ActorID <= 0 || r.OperationID != operationID || r.State == "" {
		return Receipt{}, ErrUnavailable
	}
	if pendingReceipt(r) && (!validID(r.Confirmation) || r.Expires <= 0) {
		return Receipt{}, ErrUnavailable
	}
	return r, nil
}

func (c *CoreClient) PublicConfig(ctx context.Context) (FeeConfig, error) {
	var wire struct {
		Enabled         bool            `json:"enabled"`
		Code            string          `json:"code"`
		RateBasisPoints int64           `json:"rate_basis_points"`
		RatePercent     json.RawMessage `json:"rate_percent"`
		Limits          FeeLimits       `json:"limits"`
	}
	if e := c.call(ctx, "", "config", struct{}{}, &wire); e != nil {
		return FeeConfig{}, e
	}
	if (wire.Code != "configured" && wire.Code != "policy_not_configured") || wire.Enabled != (wire.Code == "configured") || wire.RateBasisPoints < 0 || wire.RateBasisPoints > 10000 {
		return FeeConfig{}, ErrUnavailable
	}
	return FeeConfig{Enabled: wire.Enabled, Code: wire.Code, RateBasisPoints: wire.RateBasisPoints, RatePercent: json.Number(fmt.Sprintf("%d.%02d", wire.RateBasisPoints/100, wire.RateBasisPoints%100)), Limits: wire.Limits}, nil
}

// Ready is a host-only live Core policy/ACL probe, never a caller-set flag.
func (c *CoreClient) Ready(ctx context.Context) error {
	var ack struct {
		Protocol         int    `json:"protocol"`
		Service          string `json:"service"`
		Ready            bool   `json:"ready"`
		PolicyConfigured bool   `json:"policy_configured"`
	}
	var raw json.RawMessage
	if e := c.call(ctx, "", "ready", struct{}{}, &raw); e != nil {
		return e
	}
	decoder := json.NewDecoder(bytes.NewReader(raw))
	decoder.DisallowUnknownFields()
	if decoder.Decode(&ack) != nil || ack.Protocol != 1 || ack.Service != "bounties" || !ack.Ready || !ack.PolicyConfigured {
		return ErrUnavailable
	}
	return nil
}
func (s *Service) Ready(ctx context.Context) error {
	if s == nil || s.Store == nil {
		return ErrUnavailable
	}
	authority, ok := s.Funds.(interface{ Ready(context.Context) error })
	if !ok {
		return ErrUnavailable
	}
	return authority.Ready(ctx)
}
