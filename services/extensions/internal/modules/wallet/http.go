// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package wallet

import (
	"context"
	"encoding/json"
	"errors"
	"io"
	"math/big"
	"mime"
	"net"
	"net/http"
	"net/url"
	"regexp"
	"strconv"
	"strings"

	"github.com/TokenNotIncluded/lmm-api/services/extensions/internal/modules/assistant/access"
)

// TrustedSessionExtractor must be supplied by host authentication middleware.
// Return the original Core Session, never a user ID or unchecked Authorization
// value. Service independently asks Core to enforce the personal Session kind.
type TrustedSessionExtractor func(context.Context, *http.Request) (string, error)
type Handler struct {
	service *Service
	session TrustedSessionExtractor
	origin  string
	mux     *http.ServeMux
}

func NewHandler(s *Service, extract TrustedSessionExtractor, webOrigin string) (*Handler, error) {
	return NewHandlerWithLocalHTTP(s, extract, webOrigin, false)
}
func NewHandlerWithLocalHTTP(s *Service, extract TrustedSessionExtractor, webOrigin string, localHTTP bool) (*Handler, error) {
	u, e := url.Parse(webOrigin)
	local := e == nil && localHTTP && u.Scheme == "http" && net.ParseIP(u.Hostname()) != nil && net.ParseIP(u.Hostname()).IsLoopback()
	if s == nil || extract == nil || e != nil || u.Host == "" || (u.Scheme != "https" && !local) || u.User != nil || u.Path != "" || u.RawQuery != "" || u.Fragment != "" {
		return nil, ErrInvalid
	}
	h := &Handler{service: s, session: extract, origin: webOrigin, mux: http.NewServeMux()}
	h.mux.HandleFunc("GET /api/wallet/balance", h.balance)
	h.mux.HandleFunc("POST /api/wallet/topup-link", h.topup)
	h.mux.HandleFunc("GET /api/wallet-transfer", h.list)
	h.mux.HandleFunc("POST /api/wallet-transfer", h.create)
	h.mux.HandleFunc("GET /api/wallet-transfer/query", h.query)
	h.mux.HandleFunc("POST /api/wallet-transfer/inspect", func(w http.ResponseWriter, r *http.Request) { h.receipt(w, r, false) })
	h.mux.HandleFunc("POST /api/wallet-transfer/claim", func(w http.ResponseWriter, r *http.Request) { h.receipt(w, r, true) })
	h.mux.HandleFunc("POST /api/wallet-transfer/{id}/cancel", h.cancel)
	return h, nil
}
func (h *Handler) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	w.Header().Set("Cache-Control", "no-store")
	w.Header().Set("Pragma", "no-cache")
	w.Header().Set("Referrer-Policy", "no-referrer")
	w.Header().Set("X-Content-Type-Options", "nosniff")
	// Configured external Web origin is independent of the Core confirmation URL.
	// Neither Host nor forwarded headers can select a trusted origin.
	if r.Method == http.MethodPost {
		v := r.Header.Values("Origin")
		if len(v) != 1 || v[0] != h.origin {
			webError(w, ErrForbidden)
			return
		}
	}
	h.mux.ServeHTTP(w, r)
}
func (h *Handler) credential(r *http.Request) (string, error) {
	s, e := h.session(r.Context(), r)
	if e != nil {
		return "", e
	}
	if s == "" || len(s) > 256 || strings.TrimSpace(s) != s {
		return "", ErrUnauthorized
	}
	return s, nil
}

type webEnvelope struct {
	Success bool   `json:"success"`
	Message string `json:"message"`
	Code    string `json:"code,omitempty"`
	Data    any    `json:"data"`
}

func webJSON(w http.ResponseWriter, status int, v webEnvelope) {
	w.Header().Set("Content-Type", "application/json; charset=utf-8")
	w.WriteHeader(status)
	_ = json.NewEncoder(w).Encode(v)
}
func webOK(w http.ResponseWriter, data any) { webJSON(w, 200, webEnvelope{Success: true, Data: data}) }

var errWebDenominationChanged = errors.New("credit denomination changed")

func webError(w http.ResponseWriter, e error) {
	status, code, message := 503, "WALLET_RESULT_UNKNOWN", "Wallet result is unknown. Query the original request before trying again."
	switch {
	case errors.Is(e, errWebDenominationChanged):
		status, code, message = 409, "CREDIT_DENOMINATION_CHANGED", "Credit units changed; refresh before transferring"
	case errors.Is(e, ErrInvalid):
		status, code, message = 400, "WALLET_INVALID_REQUEST", "Invalid transfer amount or request"
	case errors.Is(e, ErrUnauthorized):
		status, code, message = 401, "WALLET_AUTH_REQUIRED", "Wallet authentication is required"
	case errors.Is(e, ErrForbidden):
		status, code, message = 403, "WALLET_FORBIDDEN", "Wallet action is not permitted"
	case errors.Is(e, ErrNotFound):
		status, code, message = 404, "WALLET_NOT_FOUND", "Transfer is unavailable"
	case errors.Is(e, ErrConflict):
		status, code, message = 409, "WALLET_STATE_CONFLICT", "Transfer state or immutable request conflicts"
	}
	webJSON(w, status, webEnvelope{Code: code, Message: message})
}
func webDecode(w http.ResponseWriter, r *http.Request, out any, allowed ...string) error {
	media, _, e := mime.ParseMediaType(r.Header.Get("Content-Type"))
	if e != nil || media != "application/json" {
		return ErrInvalid
	}
	raw, e := io.ReadAll(http.MaxBytesReader(w, r.Body, 4096))
	if e != nil || access.Decode(raw, out) != nil {
		return ErrInvalid
	}
	// Existing decoder checks duplicates, UTF-8, nesting, null and trailing values.
	// Require exact field spelling too, rather than Go's case-insensitive aliases.
	var fields map[string]json.RawMessage
	if json.Unmarshal(raw, &fields) != nil {
		return ErrInvalid
	}
	for key := range fields {
		ok := false
		for _, name := range allowed {
			if name == key {
				ok = true
				break
			}
		}
		if !ok {
			return ErrInvalid
		}
	}
	return nil
}
func webQuery(r *http.Request, allowed string) (string, error) {
	q, e := url.ParseQuery(r.URL.RawQuery)
	if e != nil {
		return "", ErrInvalid
	}
	if len(q) == 0 {
		return "", nil
	}
	v, ok := q[allowed]
	if allowed == "" || len(q) != 1 || !ok || len(v) != 1 || v[0] == "" {
		return "", ErrInvalid
	}
	return v[0], nil
}

const maxWebCredits int64 = 9007199254740991

func safeWebCredits(n int64) bool { return n >= -maxWebCredits && n <= maxWebCredits }

// Legacy integer-quota shape plus explicit native confirmation state. Recipient
// contacts are not invented from extension records or exposed to a claimant.
type WebTransfer struct {
	ID                        int64  `json:"id"`
	Token                     string `json:"token,omitempty"`
	Quota                     int64  `json:"quota"`
	Status                    string `json:"status"`
	CreatedAt                 int64  `json:"created_at"`
	ClaimedAt                 int64  `json:"claimed_at"`
	CancelledAt               int64  `json:"cancelled_at"`
	RequestKey                string `json:"request_key"`
	ShareURL                  string `json:"share_url,omitempty"`
	ConfirmationURL           string `json:"confirmation_url,omitempty"`
	ClaimLinkAvailable        bool   `json:"claim_link_available"`
	RecipientDetailsAvailable bool   `json:"recipient_details_available"`
	PublicCreditAmount        string `json:"public_credit_amount"`
	CreditUnitSchemaVersion   int    `json:"credit_unit_schema_version"`
	QuotaUnit                 string `json:"quota_unit"`
	PublicCreditUnit          string `json:"public_credit_unit"`
	PublicCreditsPerUSDExact  string `json:"public_credits_per_usd_exact"`
}

func (h *Handler) view(t Transfer) (WebTransfer, error) {
	if !safeWebCredits(t.ID) || !safeWebCredits(t.AmountCredits) {
		return WebTransfer{}, ErrUnknown
	}
	v := WebTransfer{ID: t.ID, Quota: t.AmountCredits, Status: t.State, CreatedAt: t.CreatedAt, RequestKey: t.RequestKey, ConfirmationURL: t.ConfirmationURL, PublicCreditAmount: strconv.FormatInt(t.AmountCredits, 10), CreditUnitSchemaVersion: 2, QuotaUnit: "LEDGER_QUOTA", PublicCreditUnit: "CREDIT", PublicCreditsPerUSDExact: "500000"}
	if t.FinishedAt != nil {
		if t.State == "claimed" {
			v.ClaimedAt = *t.FinishedAt
		} else if t.State == "cancelled" {
			v.CancelledAt = *t.FinishedAt
		}
	}
	if t.State == "pending" && t.ShareURL != "" {
		u, e := url.Parse(t.ShareURL)
		if e != nil || !validToken(u.Fragment) {
			return WebTransfer{}, ErrUnknown
		}
		v.Token = u.Fragment
		v.ShareURL = h.origin + "/transfer#" + u.Fragment
		v.ClaimLinkAvailable = true
	}
	return v, nil
}
func (h *Handler) confirmation(w http.ResponseWriter, t Transfer) {
	v, e := h.view(t)
	if e != nil {
		webError(w, e)
		return
	}
	webJSON(w, 202, webEnvelope{Code: "WALLET_CONFIRMATION_REQUIRED", Message: "Review and confirm the exact transfer in the Core wallet page.", Data: map[string]any{"confirmation_url": t.ConfirmationURL, "request_key": t.RequestKey, "transfer": v}})
}
func (h *Handler) balance(w http.ResponseWriter, r *http.Request) {
	s, e := h.credential(r)
	if e != nil {
		webError(w, e)
		return
	}
	if _, e = webQuery(r, ""); e != nil {
		webError(w, e)
		return
	}
	b, e := h.service.Balance(r.Context(), s)
	if e != nil {
		webError(w, e)
		return
	}
	for _, n := range []int64{b.AvailableCredits, b.SpendableCredits, b.ReservedCredits, b.DebtCredits, b.AffiliateCredits} {
		if !safeWebCredits(n) {
			webError(w, ErrUnknown)
			return
		}
	}
	webOK(w, map[string]any{"schema_version": 2, "available_quota": b.AvailableCredits, "available_credits": b.AvailableCredits, "spendable_credits": b.SpendableCredits, "reserved_credits": b.ReservedCredits, "debt_credits": b.DebtCredits, "affiliate_credits": b.AffiliateCredits, "credits_per_usd": CreditsPerUSD, "public_credits_per_usd_exact": "500000", "public_available_credits": strconv.FormatInt(b.AvailableCredits, 10), "currency_unit": "CREDIT"})
}
func (h *Handler) list(w http.ResponseWriter, r *http.Request) {
	s, e := h.credential(r)
	if e != nil {
		webError(w, e)
		return
	}
	raw, e := webQuery(r, "before")
	if e != nil {
		webError(w, e)
		return
	}
	before := int64(0)
	if raw != "" {
		before, e = strconv.ParseInt(raw, 10, 64)
		if e != nil || before < 0 || strconv.FormatInt(before, 10) != raw {
			webError(w, ErrInvalid)
			return
		}
	}
	rows, e := h.service.List(r.Context(), s, before)
	if e != nil {
		webError(w, e)
		return
	}
	out := make([]WebTransfer, 0, len(rows))
	for _, t := range rows {
		v, e := h.view(t)
		if e != nil {
			webError(w, e)
			return
		}
		out = append(out, v)
	}
	webOK(w, out)
}
func (h *Handler) query(w http.ResponseWriter, r *http.Request) {
	s, e := h.credential(r)
	if e != nil {
		webError(w, e)
		return
	}
	if r.URL.Query().Has("cancel_transfer_id") {
		raw, err := webQuery(r, "cancel_transfer_id")
		id, parseErr := strconv.ParseInt(raw, 10, 64)
		if err != nil || parseErr != nil || id <= 0 || strconv.FormatInt(id, 10) != raw {
			webError(w, ErrInvalid)
			return
		}
		t, err := h.service.QueryTransfer(r.Context(), s, id)
		if err != nil {
			webError(w, err)
			return
		}
		if t.State == "pending" {
			t, err = h.service.QueryCancellation(r.Context(), s, id)
			if err != nil {
				webError(w, err)
				return
			}
			h.confirmation(w, t)
			return
		}
		v, err := h.view(t)
		if err != nil {
			webError(w, err)
			return
		}
		webOK(w, v)
		return
	}
	key, e := webQuery(r, "request_key")
	if e != nil || key == "" {
		webError(w, ErrInvalid)
		return
	}
	t, e := h.service.QueryTransferByRequestKey(r.Context(), s, key)
	if e != nil {
		webError(w, e)
		return
	}
	if t.State == "awaiting_confirmation" {
		h.confirmation(w, t)
		return
	}
	v, e := h.view(t)
	if e != nil {
		webError(w, e)
		return
	}
	webOK(w, v)
}

type webCreateInput struct {
	Quota         json.RawMessage `json:"quota"`
	SchemaVersion json.RawMessage `json:"schema_version"`
	Amount        json.RawMessage `json:"amount"`
	Unit          json.RawMessage `json:"unit"`
	Expected      json.RawMessage `json:"expected_public_credits_per_usd_exact"`
	RequestKey    string          `json:"request_key"`
}

var webDecimal = regexp.MustCompile(`^([0-9]+)(?:\.([0-9]+))?(?:[eE]([+-]?[0-9]{1,3}))?$`)

func (i webCreateInput) input() (CreateInput, error) {
	out := CreateInput{RequestKey: i.RequestKey}
	if !validKey(i.RequestKey) {
		return out, ErrInvalid
	}
	versioned := len(i.SchemaVersion)+len(i.Amount)+len(i.Unit)+len(i.Expected) > 0
	if !versioned {
		if json.Unmarshal(i.Quota, &out.AmountCredits) != nil || out.AmountCredits <= 0 || !safeWebCredits(out.AmountCredits) {
			return out, ErrInvalid
		}
		return out, nil
	}
	if len(i.Quota) != 0 {
		return out, ErrInvalid
	}
	var version int
	var amount, unit string
	if json.Unmarshal(i.SchemaVersion, &version) != nil || version != 2 || json.Unmarshal(i.Amount, &amount) != nil || json.Unmarshal(i.Unit, &unit) != nil || len(amount) > 128 {
		return out, ErrInvalid
	}
	switch unit {
	case "CREDIT":
		var expected string
		if json.Unmarshal(i.Expected, &expected) != nil || expected == "" {
			return out, ErrInvalid
		}
		if expected != "500000" {
			return out, errWebDenominationChanged
		}
	case "LEDGER_QUOTA":
		if len(i.Expected) != 0 {
			return out, ErrInvalid
		}
	default:
		return out, ErrInvalid
	}
	parts := webDecimal.FindStringSubmatch(amount)
	if parts == nil || len(parts[1])+len(parts[2]) > 80 {
		return out, ErrInvalid
	}
	exponent := 0
	if parts[3] != "" {
		var e error
		exponent, e = strconv.Atoi(parts[3])
		if e != nil {
			return out, ErrInvalid
		}
	}
	exponent -= len(parts[2])
	if exponent < -18 || exponent > 18 {
		return out, ErrInvalid
	}
	rat, ok := new(big.Rat).SetString(amount)
	if !ok || rat.Sign() <= 0 {
		return out, ErrInvalid
	}
	if unit == "LEDGER_QUOTA" && !rat.IsInt() {
		return out, ErrInvalid
	}
	n := new(big.Int).Quo(rat.Num(), rat.Denom())
	if !n.IsInt64() {
		return out, ErrInvalid
	}
	out.AmountCredits = n.Int64()
	if out.AmountCredits <= 0 || !safeWebCredits(out.AmountCredits) {
		return out, ErrInvalid
	}
	return out, nil
}
func (h *Handler) create(w http.ResponseWriter, r *http.Request) {
	s, e := h.credential(r)
	if e != nil {
		webError(w, e)
		return
	}
	if _, e = webQuery(r, ""); e != nil {
		webError(w, e)
		return
	}
	var raw webCreateInput
	if e = webDecode(w, r, &raw, "quota", "schema_version", "amount", "unit", "expected_public_credits_per_usd_exact", "request_key"); e != nil {
		webError(w, e)
		return
	}
	in, e := raw.input()
	if e != nil {
		webError(w, e)
		return
	}
	t, e := h.service.Create(r.Context(), s, in)
	if e != nil {
		webError(w, e)
		return
	}
	if t.State == "awaiting_confirmation" {
		h.confirmation(w, t)
		return
	}
	v, e := h.view(t)
	if e != nil {
		webError(w, e)
		return
	}
	webOK(w, v)
}
func (h *Handler) cancel(w http.ResponseWriter, r *http.Request) {
	s, e := h.credential(r)
	if e != nil {
		webError(w, e)
		return
	}
	if _, e = webQuery(r, ""); e != nil {
		webError(w, e)
		return
	}
	id, e := strconv.ParseInt(r.PathValue("id"), 10, 64)
	if e != nil || id <= 0 || strconv.FormatInt(id, 10) != r.PathValue("id") {
		webError(w, ErrInvalid)
		return
	}
	var empty struct{}
	if e = webDecode(w, r, &empty); e != nil {
		webError(w, e)
		return
	}
	t, e := h.service.Cancel(r.Context(), s, id)
	if e != nil {
		webError(w, e)
		return
	}
	if t.State != "cancelled" {
		h.confirmation(w, t)
		return
	}
	webOK(w, nil)
}
func (h *Handler) receipt(w http.ResponseWriter, r *http.Request, claim bool) {
	s, e := h.credential(r)
	if e != nil {
		webError(w, e)
		return
	}
	if _, e = webQuery(r, ""); e != nil {
		webError(w, e)
		return
	}
	var in struct {
		Token string `json:"token"`
	}
	if e = webDecode(w, r, &in, "token"); e != nil {
		webError(w, e)
		return
	}
	var t Receipt
	if claim {
		t, e = h.service.Claim(r.Context(), s, in.Token)
	} else {
		t, e = h.service.Inspect(r.Context(), s, in.Token)
	}
	if e != nil {
		webError(w, e)
		return
	}
	if !safeWebCredits(t.AmountCredits) {
		webError(w, ErrUnknown)
		return
	}
	claimedAt := int64(0)
	if t.State == "claimed" && t.FinishedAt != nil {
		claimedAt = *t.FinishedAt
	}
	webOK(w, map[string]any{"quota": t.AmountCredits, "status": t.State, "created_at": t.CreatedAt, "claimed_at": claimedAt, "is_sender": t.IsSender, "claimed_by_me": t.ClaimedByMe, "public_credit_amount": strconv.FormatInt(t.AmountCredits, 10), "credit_unit_schema_version": 2, "public_credit_unit": "CREDIT", "public_credits_per_usd_exact": "500000"})
}
func (h *Handler) topup(w http.ResponseWriter, r *http.Request) {
	s, e := h.credential(r)
	if e != nil {
		webError(w, e)
		return
	}
	if _, e = webQuery(r, ""); e != nil {
		webError(w, e)
		return
	}
	var in struct {
		Amount int `json:"amount"`
	}
	if e = webDecode(w, r, &in, "amount"); e != nil {
		webError(w, e)
		return
	}
	t, e := h.service.TopupLink(r.Context(), s, in.Amount)
	if e != nil {
		webError(w, e)
		return
	}
	t.URL = h.origin + "/wallet?topup_amount=" + strconv.Itoa(in.Amount)
	webOK(w, t)
}
