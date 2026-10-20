// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
// Package wallet implements business records and official links, without a Core DSN.
package wallet

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"errors"
	"fmt"
	"math"
	"net"
	"net/url"
	"strconv"
	"strings"
)

var (
	ErrInvalid      = errors.New("invalid wallet request")
	ErrUnauthorized = errors.New("wallet authentication required")
	ErrForbidden    = errors.New("wallet action forbidden")
	ErrNotFound     = errors.New("wallet transfer not found")
	ErrConflict     = errors.New("wallet transfer state or immutable input conflict")
	ErrUnknown      = errors.New("wallet result unknown; retry the same request key")
)

const CreditsPerUSD int64 = 500000

// Every method authenticates the original personal Session in Core. No caller
// supplied payer, user ID, confirmed boolean or recipient ID crosses this port.
type Authority interface {
	QueryTransferByRequestKey(context.Context, string, string) (Transfer, error)
	Balance(context.Context, string) (Balance, error)
	Prepare(context.Context, string, PrepareInput) (Transfer, error)
	List(context.Context, string, int64) ([]Transfer, error)
	Cancel(context.Context, string, int64) (Transfer, error)
	Inspect(context.Context, string, string) (Receipt, error)
	Claim(context.Context, string, string) (Receipt, error)
}
type Balance struct {
	OwnerUserID       int64 `json:"owner_user_id"`
	CreditsPerUSD     int64 `json:"credits_per_usd"`
	AccountID         int64 `json:"account_id"`
	AvailableCredits  int64 `json:"available_units"`
	SpendableCredits  int64 `json:"spendable_units"`
	DebtCredits       int64 `json:"debt_units"`
	AffiliateCredits  int64 `json:"affiliate_units"`
	ReservedCredits   int64 `json:"reserved_units"`
	AvailableRevision int64 `json:"available_revision"`
	ReservedRevision  int64 `json:"reserved_revision"`
}
type CreateInput struct {
	AmountCredits int64  `json:"amount_credits"`
	RequestKey    string `json:"request_key"`
}
type PrepareInput struct {
	CreateInput
	Token string `json:"token"`
}
type Transfer struct {
	ID              int64  `json:"id"`
	RequestKey      string `json:"request_key"`
	AmountCredits   int64  `json:"amount_credits"`
	State           string `json:"state"`
	CreatedAt       int64  `json:"created_at"`
	ConfirmedAt     *int64 `json:"confirmed_at"`
	FinishedAt      *int64 `json:"finished_at"`
	ConfirmationURL string `json:"confirmation_url,omitempty"`
	ShareURL        string `json:"share_url,omitempty"`
}
type Receipt struct {
	ID            int64  `json:"id"`
	AmountCredits int64  `json:"amount_credits"`
	State         string `json:"state"`
	CreatedAt     int64  `json:"created_at"`
	FinishedAt    *int64 `json:"finished_at"`
	IsSender      bool   `json:"is_sender"`
	ClaimedByMe   bool   `json:"claimed_by_me"`
}
type Topup struct {
	URL                         string `json:"url"`
	Amount                      int    `json:"legacy_batch_amount"`
	AmountUnit                  string `json:"amount_unit"`
	PaymentConfirmationRequired bool   `json:"payment_confirmation_required"`
}

// Records retain only extension business data. Core owns immutable economics.
// PutIfAbsent MUST be durable before Prepare and atomic on (owner,key), returning
// the original token on retries and rejecting a different immutable amount.
type Records interface {
	PutIfAbsent(context.Context, int64, CreateInput, string) (string, error)
	Token(context.Context, int64, string) (string, error)
}
type Service struct {
	authority Authority
	records   Records
	origin    string
}

func New(authority Authority, records Records, origin string) (*Service, error) {
	return NewWithLocalHTTP(authority, records, origin, false)
}

// NewWithLocalHTTP allows explicit numeric loopback HTTP only for local candidates.
func NewWithLocalHTTP(authority Authority, records Records, origin string, localHTTP bool) (*Service, error) {
	u, e := url.Parse(origin)
	local := e == nil && localHTTP && u.Scheme == "http" && net.ParseIP(u.Hostname()) != nil && net.ParseIP(u.Hostname()).IsLoopback()
	if authority == nil || records == nil || e != nil || (u.Scheme != "https" && !local) || u.Host == "" || u.User != nil || u.RawQuery != "" || u.Fragment != "" || u.Path != "" || len(origin) > 512 {
		return nil, ErrInvalid
	}
	return &Service{authority, records, origin}, nil
}
func validKey(s string) bool {
	if len(s) < 16 || len(s) > 64 {
		return false
	}
	for _, b := range []byte(s) {
		if !(b >= 'a' && b <= 'z' || b >= 'A' && b <= 'Z' || b >= '0' && b <= '9' || b == '-' || b == '_') {
			return false
		}
	}
	return true
}
func validToken(s string) bool {
	if len(s) != 64 {
		return false
	}
	_, e := hex.DecodeString(s)
	return e == nil
}
func (s *Service) Balance(ctx context.Context, session string) (Balance, error) {
	if s == nil || s.authority == nil || strings.TrimSpace(session) == "" {
		return Balance{}, ErrUnauthorized
	}
	b, e := s.authority.Balance(ctx, session)
	if e != nil {
		return Balance{}, e
	}
	if b.OwnerUserID <= 0 || b.CreditsPerUSD != CreditsPerUSD || b.AccountID <= 0 {
		return Balance{}, ErrUnknown
	}
	return b, nil
}
func (s *Service) TopupLink(ctx context.Context, session string, amount int) (Topup, error) {
	if amount < 1 || amount > 1000000 {
		return Topup{}, ErrInvalid
	}
	if _, e := s.Balance(ctx, session); e != nil {
		return Topup{}, e
	}
	return Topup{URL: s.origin + "/wallet?topup_amount=" + strconv.Itoa(amount), Amount: amount, AmountUnit: "LEGACY", PaymentConfirmationRequired: true}, nil
}
func (s *Service) Create(ctx context.Context, session string, in CreateInput) (Transfer, error) {
	if in.AmountCredits <= 0 || !validKey(in.RequestKey) {
		return Transfer{}, ErrInvalid
	}
	b, e := s.Balance(ctx, session)
	if e != nil {
		return Transfer{}, e
	}
	tokenBytes := make([]byte, 32)
	if _, e = rand.Read(tokenBytes); e != nil {
		return Transfer{}, ErrUnknown
	}
	token, e := s.records.PutIfAbsent(ctx, b.OwnerUserID, in, hex.EncodeToString(tokenBytes))
	if e != nil {
		return Transfer{}, e
	}
	if !validToken(token) {
		return Transfer{}, ErrUnknown
	}
	// A timeout is UNKNOWN: never choose another request key/token or compensate.
	t, e := s.authority.Prepare(ctx, session, PrepareInput{in, token})
	if e != nil {
		return Transfer{}, e
	}
	if t.RequestKey != in.RequestKey || t.AmountCredits != in.AmountCredits {
		return Transfer{}, ErrUnknown
	}
	return s.view(t, token)
}
func (s *Service) view(t Transfer, token string) (Transfer, error) {
	if t.ID <= 0 || t.AmountCredits <= 0 || !validKey(t.RequestKey) {
		return Transfer{}, ErrUnknown
	}
	switch t.State {
	case "awaiting_confirmation":
		t.ConfirmationURL = fmt.Sprintf("%s/core/v1/wallet/confirm/create/%d", s.origin, t.ID)
	case "pending":
		if validToken(token) {
			t.ShareURL = s.origin + "/transfer#" + token
		}
	case "claimed", "cancelled":
	default:
		return Transfer{}, ErrUnknown
	}
	return t, nil
}
func (s *Service) List(ctx context.Context, session string, before int64) ([]Transfer, error) {
	if before < 0 {
		return nil, ErrInvalid
	}
	b, e := s.Balance(ctx, session)
	if e != nil {
		return nil, e
	}
	rows, e := s.authority.List(ctx, session, before)
	if e != nil {
		return nil, e
	}
	if len(rows) > 50 {
		return nil, ErrUnknown
	}
	out := make([]Transfer, 0, len(rows))
	for _, t := range rows {
		token := ""
		if t.State == "pending" {
			token, e = s.records.Token(ctx, b.OwnerUserID, t.RequestKey)
			if e != nil && !errors.Is(e, ErrNotFound) {
				return nil, e
			}
		}
		v, e := s.view(t, token)
		if e != nil {
			return nil, e
		}
		out = append(out, v)
	}
	return out, nil
}

// Cancel requests an exact Core browser confirmation. Only the browser POST can
// release the original hold. A concurrent claim makes that confirmation fail.
func (s *Service) Cancel(ctx context.Context, session string, id int64) (Transfer, error) {
	if id <= 0 {
		return Transfer{}, ErrInvalid
	}
	if _, e := s.Balance(ctx, session); e != nil {
		return Transfer{}, e
	}
	t, e := s.authority.Cancel(ctx, session, id)
	if e != nil {
		return Transfer{}, e
	}
	if t.ID != id {
		return Transfer{}, ErrUnknown
	}
	t, e = s.view(t, "")
	if e != nil {
		return Transfer{}, e
	}
	if t.State == "pending" {
		t.ConfirmationURL = fmt.Sprintf("%s/core/v1/wallet/confirm/cancel/%d", s.origin, id)
	} else if t.State != "cancelled" {
		return Transfer{}, ErrConflict
	}
	return t, nil
}
func (s *Service) Inspect(ctx context.Context, session, token string) (Receipt, error) {
	if !validToken(token) {
		return Receipt{}, ErrInvalid
	}
	if _, e := s.Balance(ctx, session); e != nil {
		return Receipt{}, e
	}
	r, e := s.authority.Inspect(ctx, session, token)
	if e != nil {
		return Receipt{}, e
	}
	if !validReceipt(r) {
		return Receipt{}, ErrUnknown
	}
	return r, nil
}
func (s *Service) Claim(ctx context.Context, recipientSession, token string) (Receipt, error) {
	if !validToken(token) {
		return Receipt{}, ErrInvalid
	}
	if _, e := s.Balance(ctx, recipientSession); e != nil {
		return Receipt{}, e
	}
	r, e := s.authority.Claim(ctx, recipientSession, token)
	if e != nil {
		return Receipt{}, e
	}
	if !validReceipt(r) || r.State != "claimed" || !r.ClaimedByMe || r.IsSender {
		return Receipt{}, ErrUnknown
	}
	return r, nil
}

// QueryTransfer recovers only an existing owner transfer. Core List authenticates
// and scopes the cursor to this sender. id+1 makes an existing exact ID the first
// eligible row, so the 50-row limit cannot hide it behind newer records.
// It never invokes Prepare, Cancel, or a financial operation.
func (s *Service) QueryTransfer(ctx context.Context, session string, id int64) (Transfer, error) {
	if id <= 0 {
		return Transfer{}, ErrInvalid
	}
	before := int64(0)
	if id < math.MaxInt64 {
		before = id + 1
	}
	rows, err := s.List(ctx, session, before)
	if err != nil {
		return Transfer{}, err
	}
	for _, row := range rows {
		if row.ID == id {
			return row, nil
		}
	}
	return Transfer{}, ErrNotFound
}

// QueryCancellation adds only the pending cancellation continuation. It does
// not request or perform a cancellation, and pending is never called cancelled.
func (s *Service) QueryCancellation(ctx context.Context, session string, id int64) (Transfer, error) {
	t, err := s.QueryTransfer(ctx, session, id)
	if err != nil {
		return Transfer{}, err
	}
	t.ShareURL = ""
	switch t.State {
	case "pending":
		t.ConfirmationURL = fmt.Sprintf("%s/core/v1/wallet/confirm/cancel/%d", s.origin, id)
	case "cancelled":
		t.ConfirmationURL = ""
	default:
		return Transfer{}, ErrConflict
	}
	return t, nil
}

// QueryTransferByRequestKey recovers the original Create when its first response
// was lost before the host learned its numeric ID. Neither a missing Core row nor
// a missing local bearer permits another Create: the result remains UNKNOWN.
func (s *Service) QueryTransferByRequestKey(ctx context.Context, session, key string) (Transfer, error) {
	if !validKey(key) {
		return Transfer{}, ErrInvalid
	}
	b, err := s.Balance(ctx, session)
	if err != nil {
		return Transfer{}, err
	}
	t, err := s.authority.QueryTransferByRequestKey(ctx, session, key)
	if errors.Is(err, ErrNotFound) {
		return Transfer{}, ErrUnknown
	}
	if err != nil {
		return Transfer{}, err
	}
	if t.RequestKey != key {
		return Transfer{}, ErrUnknown
	}
	token := ""
	if t.State == "pending" {
		token, err = s.records.Token(ctx, b.OwnerUserID, key)
		if err != nil || !validToken(token) {
			return Transfer{}, ErrUnknown
		}
	}
	return s.view(t, token)
}

func validReceipt(r Receipt) bool {
	return r.ID > 0 && r.AmountCredits > 0 && (r.State == "pending" || r.State == "claimed" || r.State == "cancelled") && !(r.IsSender && r.ClaimedByMe)
}

// Ready requires a real authority readiness port; ordinary fixture/business
// authority implementations cannot advertise startup readiness by existing.
func (s *Service) Ready(ctx context.Context) error {
	authority, ok := s.authority.(interface{ Ready(context.Context) error })
	if !ok {
		return ErrUnknown
	}
	return authority.Ready(ctx)
}
