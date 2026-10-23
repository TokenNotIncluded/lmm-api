// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
// Package rustbridge consumes the Core native tool-market v1 contract over a
// dedicated Unix socket. It has no Core DSN, supplier token or credit authority.
package rustbridge

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"io"
	"net"
	"net/http"
	"path/filepath"
	"strconv"
	"strings"
	"time"

	market "github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/toolmarket"
)

type Adapter struct {
	legacyReady          legacyReadinessState
	client               *http.Client
	token                string
	socket               string
	fixedWitness         *Adapter
	fixedWitnessReviewed bool
}
type receipt struct {
	OperationID string `json:"operation_id"`
	Reservation string `json:"reservation"`
	State       string `json:"state"`
	Credits     int64  `json:"credits"`
	JournalID   int64  `json:"journal_id"`
}

// New only accepts a dedicated internal Unix socket. The caller reads the
// separately permissioned toolmarket host token from its private secret file.
func New(socket string, token []byte) (*Adapter, error) {
	if !filepath.IsAbs(socket) || filepath.Clean(socket) != socket || strings.ContainsAny(socket, "\x00\r\n") || len(token) < 32 || len(token) > 256 {
		return nil, market.ErrInvalid
	}
	for _, b := range token {
		if b < 33 || b > 126 {
			return nil, market.ErrInvalid
		}
	}
	dialer := &net.Dialer{Timeout: 2 * time.Second}
	transport := &http.Transport{DialContext: func(ctx context.Context, _, _ string) (net.Conn, error) {
		return dialer.DialContext(ctx, "unix", socket)
	}, MaxConnsPerHost: 4, MaxIdleConnsPerHost: 2, IdleConnTimeout: 30 * time.Second, ResponseHeaderTimeout: 2 * time.Second}
	return &Adapter{socket: socket, client: &http.Client{Transport: transport, Timeout: 3 * time.Second, CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}, token: string(token)}, nil
}
func (a *Adapter) Close() {
	if a != nil && a.fixedWitness != nil {
		a.fixedWitness.Close()
	}
	if a != nil && a.client != nil {
		a.client.CloseIdleConnections()
	}
}

func (a *Adapter) request(ctx context.Context, action, user string, in, out any) error {
	return a.requestVersion(ctx, 1, action, user, in, out)
}
func (a *Adapter) requestVersion(ctx context.Context, major int, action, user string, in, out any) error {
	if a == nil || a.client == nil {
		return market.ErrUnavailable
	}
	raw, err := json.Marshal(in)
	requestLimit := 16384
	if major == 2 {
		requestLimit = 32768
	}
	if err != nil || len(raw) > requestLimit {
		return market.ErrInvalid
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, "http://core-toolmarket/internal/tool-market/v"+strconv.Itoa(major)+"/"+action, bytes.NewReader(raw))
	if err != nil {
		return market.ErrUnavailable
	}
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Authorization", "Bearer "+a.token)
	req.Header.Set("X-Lmm-Protocol", strconv.Itoa(major))
	if user != "" {
		req.Header.Set("X-Lmm-User-Credential", user)
	}
	response, err := a.client.Do(req)
	if err != nil {
		return market.ErrUnavailable
	}
	defer response.Body.Close()
	limit := int64(65536)
	if major == 2 {
		limit = 256 << 10
	}
	if major == 2 && action == "reports/list" {
		limit = 4 << 20
	}
	body, err := io.ReadAll(io.LimitReader(response.Body, limit+1))
	if err != nil || int64(len(body)) > limit {
		return market.ErrUnavailable
	}
	if response.StatusCode != http.StatusOK {
		switch response.StatusCode {
		case 400:
			return market.ErrInvalid
		case 401:
			return market.ErrAuth
		case 403:
			return market.ErrDenied
		case 404:
			return market.ErrMissing
		case 409:
			return market.ErrConflict
		default:
			return market.ErrUnavailable
		}
	}
	decoder := json.NewDecoder(bytes.NewReader(body))
	decoder.DisallowUnknownFields()
	if err = decoder.Decode(out); err != nil {
		return market.ErrUnavailable
	}
	var extra any
	if !errors.Is(decoder.Decode(&extra), io.EOF) {
		return market.ErrUnavailable
	}
	return nil
}

// Ready negotiates the complete fixed-price capability set before host mounting.
func (a *Adapter) Ready(ctx context.Context) error {
	var capability struct {
		ProtocolMajor int      `json:"protocol_major"`
		Policy        string   `json:"policy"`
		Features      []string `json:"features"`
	}
	if err := a.request(ctx, "capabilities", "", struct{}{}, &capability); err != nil {
		return err
	}
	if capability.ProtocolMajor != 1 || capability.Policy != "exact-usd-micros-500k-v1" {
		return market.ErrUnavailable
	}
	present := map[string]bool{}
	for _, feature := range capability.Features {
		if present[feature] {
			return market.ErrUnavailable
		}
		present[feature] = true
	}
	for _, feature := range []string{"authorize", "quotes", "confirm", "reserve", "dispatch", "commit", "release", "query", "reconcile", "revoke"} {
		if !present[feature] {
			return market.ErrUnavailable
		}
	}
	return nil
}

func personal(p market.Principal) bool {
	id, err := strconv.ParseInt(p.User, 10, 64)
	return err == nil && id > 0 && p.User == strconv.FormatInt(id, 10) && p.Account == p.User
}
func (a *Adapter) Authorize(ctx context.Context, user, account, action string) (market.Principal, error) {
	switch action {
	case "toolmarket.read", "toolmarket.manage", "toolmarket.authorize", "toolmarket.execute", "toolmarket.review", "toolmarket.config":
	default:
		return market.Principal{}, market.ErrDenied
	}
	var p market.Principal
	if err := a.request(ctx, "authorize", user, struct {
		Account string `json:"account"`
		Action  string `json:"action"`
	}{account, action}, &p); err != nil {
		return p, err
	}
	if !personal(p) || p.Account != account {
		return market.Principal{}, market.ErrDenied
	}
	return p, nil
}
func (a *Adapter) VerifyToolConfirmation(ctx context.Context, user string, intent market.ConfirmationIntent) (market.ConfirmationEvidence, error) {
	var evidence market.ConfirmationEvidence
	err := a.request(ctx, "confirm", user, intent, &evidence)
	return evidence, err // Module independently checks owner, exact digest and lifetime.
}
func (a *Adapter) RegisterToolQuote(ctx context.Context, user string, quote market.QuoteRegistration) error {
	var saved market.QuoteRegistration
	if err := a.request(ctx, "quotes", user, quote, &saved); err != nil {
		return err
	}
	if saved != quote {
		return market.ErrConflict
	}
	return nil
}
func checkReceipt(r *receipt, c market.Charge) error {
	if r == nil || r.OperationID != c.ID || c.Currency != "USD" || c.Amount < 0 || c.Amount%2 != 0 || r.Credits != c.Amount/2 {
		return market.ErrPending
	}
	switch r.State {
	case "reserved", "dispatched", "committed", "released":
	default:
		return market.ErrPending
	}
	if c.Amount == 0 {
		if r.JournalID != 0 || r.Reservation != "tool-market:free:"+c.ID {
			return market.ErrPending
		}
	} else {
		journal, err := strconv.ParseInt(strings.TrimPrefix(r.Reservation, "tool-market:journal:"), 10, 64)
		if r.JournalID <= 0 || err != nil || journal <= 0 || r.Reservation != "tool-market:journal:"+strconv.FormatInt(journal, 10) {
			return market.ErrPending
		}
	}
	return nil
}
func (a *Adapter) reconcile(ctx context.Context, c market.Charge) (*receipt, error) {
	var r *receipt
	if err := a.request(ctx, "reconcile", "", c, &r); err != nil {
		return nil, err
	}
	if err := checkReceipt(r, c); err != nil {
		return nil, err
	}
	return r, nil
}
func (a *Adapter) Reserve(ctx context.Context, user string, p market.Principal, c market.Charge) error {
	if !personal(p) {
		return market.ErrDenied
	}
	var r receipt
	if err := a.request(ctx, "reserve", user, c, &r); err != nil {
		return err
	}
	if err := checkReceipt(&r, c); err != nil {
		return err
	}
	if r.State != "reserved" {
		return market.ErrPending
	}
	return nil
}
func (a *Adapter) MarkDispatched(ctx context.Context, user string, p market.Principal, c market.Charge) error {
	if !personal(p) {
		return market.ErrDenied
	}
	r, err := a.reconcile(ctx, c)
	if err != nil {
		return err
	}
	if r.State != "reserved" && r.State != "dispatched" {
		return market.ErrPending
	}
	var saved receipt
	in := struct {
		OperationID string `json:"operation_id"`
		Reservation string `json:"reservation"`
	}{c.ID, r.Reservation}
	if err = a.request(ctx, "dispatch", user, in, &saved); err != nil {
		return err
	}
	if err = checkReceipt(&saved, c); err != nil {
		return err
	}
	if saved.State != "dispatched" || saved.Reservation != r.Reservation {
		return market.ErrPending
	}
	return nil
}

// Commit without a stored result hash cannot attest external completion.
func (a *Adapter) Commit(context.Context, string, market.Principal, market.Charge) error {
	return market.ErrInvalid
}
func (a *Adapter) CommitResult(ctx context.Context, _ string, p market.Principal, c market.Charge, resultHash string) error {
	if !personal(p) || len(resultHash) != 64 {
		return market.ErrInvalid
	}
	r, err := a.reconcile(ctx, c)
	if err != nil {
		return err
	}
	if r.State != "dispatched" && r.State != "committed" {
		return market.ErrPending
	}
	var saved receipt
	in := struct {
		OperationID string `json:"operation_id"`
		Reservation string `json:"reservation"`
		ResultHash  string `json:"result_hash"`
	}{c.ID, r.Reservation, resultHash}
	if err = a.request(ctx, "commit", "", in, &saved); err != nil {
		return err
	}
	if err = checkReceipt(&saved, c); err != nil {
		return err
	}
	if saved.State != "committed" || saved.Reservation != r.Reservation {
		return market.ErrPending
	}
	return nil
}

func (a *Adapter) RevokeToolApproval(ctx context.Context, user string, p market.Principal, approval string) error {
	if !personal(p) || len(approval) != 64 {
		return market.ErrInvalid
	}
	var result struct {
		Revoked bool `json:"revoked"`
	}
	if err := a.request(ctx, "revoke", user, struct {
		Approval string `json:"approval"`
	}{approval}, &result); err != nil {
		return err
	}
	if !result.Revoked {
		return market.ErrPending
	}
	return nil
}

func (a *Adapter) ReleaseUndispatched(ctx context.Context, _ string, p market.Principal, c market.Charge) error {
	if !personal(p) {
		return market.ErrDenied
	}
	original, err := a.reconcile(ctx, c)
	if err != nil {
		return err
	}
	if original.State != "reserved" && original.State != "released" {
		return market.ErrPending
	}
	var saved receipt
	in := struct {
		OperationID string `json:"operation_id"`
		Reservation string `json:"reservation"`
	}{c.ID, original.Reservation}
	if err = a.request(ctx, "release", "", in, &saved); err != nil {
		return err
	}
	if err = checkReceipt(&saved, c); err != nil {
		return err
	}
	if saved.State != "released" || saved.Reservation != original.Reservation {
		return market.ErrPending
	}
	return nil
}

var _ market.ReleaseFunds = (*Adapter)(nil)

var _ market.GrantRevoker = (*Adapter)(nil)

var _ market.Authority = (*Adapter)(nil)
var _ market.ConfirmationAuthority = (*Adapter)(nil)
var _ market.QuoteRegistrar = (*Adapter)(nil)
var _ market.Funds = (*Adapter)(nil)
var _ market.DispatchFunds = (*Adapter)(nil)
var _ market.ResultFunds = (*Adapter)(nil)
