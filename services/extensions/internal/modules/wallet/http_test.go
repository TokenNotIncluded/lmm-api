package wallet

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

type walletSessionKey struct{}

func verifiedTestSession(_ context.Context, r *http.Request) (string, error) {
	s, _ := r.Context().Value(walletSessionKey{}).(string)
	if s == "" {
		return "", ErrUnauthorized
	}
	return s, nil
}
func webRequest(method, path, body, origin string, verified bool) *http.Request {
	r := httptest.NewRequest(method, path, strings.NewReader(body))
	r.Header.Set("Content-Type", "application/json")
	if origin != "" {
		r.Header.Set("Origin", origin)
	}
	r.Header.Set("Authorization", "Bearer attacker-api-key")
	r.Header.Set("Host", "evil.example")
	r.Header.Set("X-Forwarded-Host", "evil.example")
	if verified {
		r = r.WithContext(context.WithValue(r.Context(), walletSessionKey{}, "live-session"))
	}
	return r
}
func responseJSON(t *testing.T, w *httptest.ResponseRecorder) map[string]any {
	t.Helper()
	var v map[string]any
	if e := json.Unmarshal(w.Body.Bytes(), &v); e != nil {
		t.Fatal(w.Code, w.Body.String(), e)
	}
	return v
}
func TestWebCreateAndCancelKeepCoreConfirmationAndIndependentWebOrigin(t *testing.T) {
	s, c, _ := setup(t)
	h, e := NewHandler(s, verifiedTestSession, "https://web.example")
	if e != nil {
		t.Fatal(e)
	}
	key := strings.Repeat("k", 16)
	call := func(path, body string) *httptest.ResponseRecorder {
		w := httptest.NewRecorder()
		h.ServeHTTP(w, webRequest("POST", path, body, "https://web.example", true))
		return w
	}
	w := call("/api/wallet-transfer", `{"quota":125,"request_key":"`+key+`"}`)
	v := responseJSON(t, w)
	if w.Code != 202 || v["success"] != false || v["code"] != "WALLET_CONFIRMATION_REQUIRED" {
		t.Fatal(w.Code, v)
	}
	data := v["data"].(map[string]any)
	if data["confirmation_url"] != "https://wallet.example/core/v1/wallet/confirm/create/17" || data["request_key"] != key {
		t.Fatal(data)
	}
	if w.Header().Get("Cache-Control") != "no-store" || strings.Contains(w.Body.String(), "evil.example") {
		t.Fatal("unsafe response")
	}
	c.state = "pending"
	w = call("/api/wallet-transfer", `{"quota":125,"request_key":"`+key+`"}`)
	v = responseJSON(t, w)
	data = v["data"].(map[string]any)
	if w.Code != 200 || v["success"] != true || !strings.HasPrefix(data["share_url"].(string), "https://web.example/transfer#") {
		t.Fatal(w.Code, v)
	}
	w = call("/api/wallet-transfer/17/cancel", `{}`)
	v = responseJSON(t, w)
	data = v["data"].(map[string]any)
	if w.Code != 202 || v["success"] != false || data["confirmation_url"] != "https://wallet.example/core/v1/wallet/confirm/cancel/17" {
		t.Fatal(w.Code, v)
	}
	c.state = "cancelled"
	w = call("/api/wallet-transfer/17/cancel", `{}`)
	v = responseJSON(t, w)
	if w.Code != 200 || v["success"] != true {
		t.Fatal(w.Code, v)
	}
}
func TestWebRefusesUntrustedIdentityOriginAndAmbiguousFinancialJSON(t *testing.T) {
	s, c, _ := setup(t)
	h, e := NewHandler(s, verifiedTestSession, "https://web.example")
	if e != nil {
		t.Fatal(e)
	}
	good := `{"quota":125,"request_key":"wallet-request-01"}`
	for _, tc := range []struct {
		origin   string
		verified bool
		body     string
		status   int
	}{
		{"https://evil.example", true, good, 403}, {"", true, good, 403}, {"https://web.example", false, good, 401},
		{"https://web.example", true, `{"quota":125,"request_key":"wallet-request-01","confirmed":true}`, 400},
		{"https://web.example", true, `{"quota":125,"quota":1,"request_key":"wallet-request-01"}`, 400},
		{"https://web.example", true, `{"Quota":125,"request_key":"wallet-request-01"}`, 400},
		{"https://web.example", true, `{"quota":125,"request_key":"wallet-request-01","payer_account_id":1}`, 400},
		{"https://web.example", true, good + ` {}`, 400},
	} {
		w := httptest.NewRecorder()
		h.ServeHTTP(w, webRequest("POST", "/api/wallet-transfer", tc.body, tc.origin, tc.verified))
		if w.Code != tc.status {
			t.Fatal(tc, w.Code, w.Body.String())
		}
	}
	if c.calls != 0 {
		t.Fatal("rejected body reached prepare", c.calls)
	}
	for _, bad := range []string{"http://web.example", "https://web.example/", "https://user@web.example", "https://web.example?x=y"} {
		if _, e := NewHandler(s, verifiedTestSession, bad); e == nil {
			t.Fatal(bad)
		}
	}
	if _, e = NewHandlerWithLocalHTTP(s, verifiedTestSession, "http://127.0.0.1:18080", true); e != nil {
		t.Fatal(e)
	}
	if _, e = NewHandlerWithLocalHTTP(s, verifiedTestSession, "http://localhost:18080", true); e == nil {
		t.Fatal("non-numeric local origin")
	}
}
func TestWebExplicitCreditDenominationIsExactAndFloorsOnlyPublicDust(t *testing.T) {
	s, c, _ := setup(t)
	h, _ := NewHandler(s, verifiedTestSession, "https://web.example")
	for _, tc := range []struct {
		body   string
		status int
		amount int64
	}{
		{`{"schema_version":2,"amount":"125.999999999999999999","unit":"CREDIT","expected_public_credits_per_usd_exact":"500000","request_key":"wallet-explicit-01"}`, 202, 125},
		{`{"schema_version":2,"amount":"125.1","unit":"LEDGER_QUOTA","request_key":"wallet-explicit-02"}`, 400, 0},
		{`{"schema_version":2,"amount":"1e999","unit":"CREDIT","expected_public_credits_per_usd_exact":"500000","request_key":"wallet-explicit-03"}`, 400, 0},
		{`{"schema_version":2,"amount":"125","unit":"CREDIT","expected_public_credits_per_usd_exact":"1","request_key":"wallet-explicit-04"}`, 409, 0},
		{`{"quota":125,"schema_version":2,"amount":"125","unit":"CREDIT","expected_public_credits_per_usd_exact":"500000","request_key":"wallet-explicit-05"}`, 400, 0},
	} {
		w := httptest.NewRecorder()
		h.ServeHTTP(w, webRequest("POST", "/api/wallet-transfer", tc.body, "https://web.example", true))
		if w.Code != tc.status {
			t.Fatal(tc, w.Code, w.Body.String())
		}
		if tc.amount > 0 {
			v := responseJSON(t, w)
			transfer := v["data"].(map[string]any)["transfer"].(map[string]any)
			if transfer["quota"] != float64(tc.amount) {
				t.Fatal(v)
			}
		}
	}
	if c.calls != 1 {
		t.Fatal("invalid financial input reached authority", c.calls)
	}
}

type webCore struct {
	*queryKeyCore
	claimSession   string
	inspectSession string
}

func (c *webCore) Claim(_ context.Context, s, token string) (Receipt, error) {
	c.claimSession = s
	return Receipt{ID: 17, AmountCredits: 125, State: "claimed", ClaimedByMe: true}, nil
}
func (c *webCore) Inspect(_ context.Context, s, token string) (Receipt, error) {
	c.inspectSession = s
	return Receipt{ID: 17, AmountCredits: 125, State: "pending"}, nil
}
func TestWebKeyRecoveryBalanceTopupAndRecipientReceipt(t *testing.T) {
	base, c, records := setup(t)
	key := strings.Repeat("f", 64)
	records.token = strings.Repeat("a", 64)
	core := &webCore{queryKeyCore: &queryKeyCore{queryCore: &queryCore{testCore: c}, value: Transfer{ID: 17, RequestKey: key, AmountCredits: 125, State: "awaiting_confirmation"}}}
	s, e := New(core, records, base.origin)
	if e != nil {
		t.Fatal(e)
	}
	h, _ := NewHandler(s, verifiedTestSession, "https://web.example")
	w := httptest.NewRecorder()
	h.ServeHTTP(w, webRequest("GET", "/api/wallet-transfer/query?request_key="+key, "", "", true))
	if w.Code != 202 {
		t.Fatal(w.Code, w.Body.String())
	}
	core.err = ErrNotFound
	w = httptest.NewRecorder()
	h.ServeHTTP(w, webRequest("GET", "/api/wallet-transfer/query?request_key="+key, "", "", true))
	if w.Code != 503 || responseJSON(t, w)["code"] != "WALLET_RESULT_UNKNOWN" {
		t.Fatal(w.Code, w.Body.String())
	}
	core.err = nil
	w = httptest.NewRecorder()
	h.ServeHTTP(w, webRequest("GET", "/api/wallet/balance", "", "", true))
	if w.Code != 200 || responseJSON(t, w)["data"].(map[string]any)["credits_per_usd"] != float64(CreditsPerUSD) {
		t.Fatal(w.Code, w.Body.String())
	}
	w = httptest.NewRecorder()
	h.ServeHTTP(w, webRequest("POST", "/api/wallet/topup-link", `{"amount":3}`, "https://web.example", true))
	if w.Code != 200 || responseJSON(t, w)["data"].(map[string]any)["url"] != "https://web.example/wallet?topup_amount=3" {
		t.Fatal(w.Code, w.Body.String())
	}
	token := strings.Repeat("b", 64)
	for _, path := range []string{"inspect", "claim"} {
		w = httptest.NewRecorder()
		h.ServeHTTP(w, webRequest("POST", "/api/wallet-transfer/"+path, `{"token":"`+token+`"}`, "https://web.example", true))
		if w.Code != 200 {
			t.Fatal(w.Code, w.Body.String())
		}
		body := w.Body.String()
		if strings.Contains(body, token) || strings.Contains(body, "recipient_id") || strings.Contains(body, "owner_user_id") {
			t.Fatal("receipt disclosed bearer or identity", body)
		}
	}
	if core.claimSession != "live-session" || core.inspectSession != "live-session" {
		t.Fatal("recipient Session not propagated")
	}
}

func TestWebCancellationRecoveryOnlyReadsOriginalTransfer(t *testing.T) {
	s, c, records := setup(t)
	q := &queryCore{testCore: c}
	var err error
	s, err = New(q, records, "https://wallet.example")
	if err != nil {
		t.Fatal(err)
	}
	h, e := NewHandler(s, verifiedTestSession, "https://web.example")
	if e != nil {
		t.Fatal(e)
	}
	key := strings.Repeat("k", 16)
	records.token = strings.Repeat("a", 64)
	for _, state := range []string{"pending", "cancelled", "claimed"} {
		c.state = state
		q.rows = []Transfer{{ID: 17, RequestKey: key, AmountCredits: 125, State: state}}
		w := httptest.NewRecorder()
		h.ServeHTTP(w, webRequest("GET", "/api/wallet-transfer/query?cancel_transfer_id=17", "", "", true))
		v := responseJSON(t, w)
		if state == "pending" {
			if w.Code != 202 || v["success"] != false {
				t.Fatal(state, w.Code, v)
			}
			d := v["data"].(map[string]any)
			if d["confirmation_url"] != "https://wallet.example/core/v1/wallet/confirm/cancel/17" {
				t.Fatal(d)
			}
		} else if w.Code != 200 || v["data"].(map[string]any)["status"] != state {
			t.Fatal(state, w.Code, v)
		}
	}
	w := httptest.NewRecorder()
	h.ServeHTTP(w, webRequest("GET", "/api/wallet-transfer/query?cancel_transfer_id=17&request_key="+key, "", "", true))
	if w.Code != 400 {
		t.Fatal(w.Code, w.Body.String())
	}
}
