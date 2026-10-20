package wallet

import (
	"context"
	"io"
	"net/http"
	"strings"
	"testing"
)

type roundTripFunc func(*http.Request) (*http.Response, error)

func (f roundTripFunc) RoundTrip(r *http.Request) (*http.Response, error) { return f(r) }
func TestTransportPreservesAuthorityAndDoesNotRetryUnknown(t *testing.T) {
	calls := 0
	c := &CoreClient{hostToken: strings.Repeat("x", 32)}
	c.client = &http.Client{Transport: roundTripFunc(func(r *http.Request) (*http.Response, error) {
		calls++
		if r.URL.Path != "/internal/wallet/v1/prepare" || r.Header.Get("Authorization") != "Bearer "+strings.Repeat("x", 32) || r.Header.Get("X-Lmm-User-Credential") != "personal-session" {
			t.Fatal("authority binding missing")
		}
		raw, e := io.ReadAll(r.Body)
		if e != nil || !strings.Contains(string(raw), `"request_key":"wallet-key-000001"`) || strings.Contains(string(raw), "payer") {
			t.Fatal("invalid prepare envelope")
		}
		return &http.Response{StatusCode: 503, Body: io.NopCloser(strings.NewReader(`{"error":"unknown"}`)), Header: make(http.Header)}, nil
	})}
	_, e := c.Prepare(context.Background(), "personal-session", PrepareInput{CreateInput{100, "wallet-key-000001"}, strings.Repeat("a", 64)})
	if e != ErrUnknown || calls != 1 {
		t.Fatal("transport retried or hid UNKNOWN", e, calls)
	}
}
func TestTransportRejectsAuthorityExpansionAndMalformedReceipts(t *testing.T) {
	c := &CoreClient{hostToken: strings.Repeat("x", 32)}
	for _, body := range []string{`{"owner_user_id":1,"credits_per_usd":500000,"payer":5}`, `{} {}`, `not-json`} {
		c.client = &http.Client{Transport: roundTripFunc(func(*http.Request) (*http.Response, error) {
			return &http.Response{StatusCode: 200, Body: io.NopCloser(strings.NewReader(body)), Header: make(http.Header)}, nil
		})}
		if _, e := c.Balance(context.Background(), "session"); e != ErrUnknown {
			t.Fatal("untrusted response accepted", body, e)
		}
	}
	for _, s := range []string{"", "session\r\nx-injected: true"} {
		if _, e := c.Balance(context.Background(), s); e != ErrUnauthorized {
			t.Fatal("invalid session accepted", e)
		}
	}
}

func TestReadinessIsHostOnlyProtocolOneAndEmptyRequest(t *testing.T) {
	calls := 0
	c := &CoreClient{hostToken: strings.Repeat("x", 32)}
	c.client = &http.Client{Transport: roundTripFunc(func(r *http.Request) (*http.Response, error) {
		calls++
		body, e := io.ReadAll(r.Body)
		if e != nil || r.Method != "POST" || r.URL.Path != "/internal/wallet/v1/ready" || string(body) != "{}" || r.Header.Get("Authorization") != "Bearer "+strings.Repeat("x", 32) || r.Header.Get("X-Lmm-Protocol") != "1" || r.Header.Get("X-Lmm-User-Credential") != "" {
			t.Fatal("invalid readiness authority", r.Header, string(body))
		}
		return &http.Response{StatusCode: 200, Body: io.NopCloser(strings.NewReader(`{"protocol":1,"service":"wallet","ready":true}`)), Header: make(http.Header)}, nil
	})}
	if e := c.Ready(context.Background()); e != nil || calls != 1 {
		t.Fatal(e, calls)
	}
}
func TestReadinessRejectsMissingFalseMalformedOrExpandedACK(t *testing.T) {
	c := &CoreClient{hostToken: strings.Repeat("x", 32)}
	for _, body := range []string{`{}`, `null`, `{"protocol":1,"service":"wallet","ready":false}`, `{"protocol":2,"service":"wallet","ready":true}`, `{"protocol":1,"service":"other","ready":true}`, `{"protocol":1,"service":"wallet","ready":true,"extra":true}`, `{"protocol":1,"service":"wallet","ready":false,"ready":true}`, `{"Protocol":1,"service":"wallet","ready":true}`, `{"protocol":1,"service":"wallet","ready":true} {}`} {
		c.client = &http.Client{Transport: roundTripFunc(func(*http.Request) (*http.Response, error) {
			return &http.Response{StatusCode: 200, Body: io.NopCloser(strings.NewReader(body)), Header: make(http.Header)}, nil
		})}
		if e := c.Ready(context.Background()); e != ErrUnknown {
			t.Fatal(body, e)
		}
	}
	for _, status := range []int{401, 403, 503, 302} {
		c.client = &http.Client{Transport: roundTripFunc(func(*http.Request) (*http.Response, error) {
			return &http.Response{StatusCode: status, Body: io.NopCloser(strings.NewReader(`{"protocol":1,"service":"wallet","ready":true}`)), Header: make(http.Header)}, nil
		})}
		if e := c.Ready(context.Background()); e != ErrUnknown {
			t.Fatal(status, e)
		}
	}
}
