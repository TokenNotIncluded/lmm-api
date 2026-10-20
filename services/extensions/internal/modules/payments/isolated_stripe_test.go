package payments

import (
	"context"
	"crypto/rand"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"net"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"testing"
)

func isolatedStripeFixture(t *testing.T) (StripeIsolatedConfig, *httptest.Server, *http.ServeMux) {
	t.Helper()
	// Do not rely on :0: Linux ephemeral allocation includes protected low ports.
	var listener net.Listener
	for i := 0; i < 1024; i++ {
		var random [2]byte
		if _, err := rand.Read(random[:]); err != nil {
			t.Fatal(err)
		}
		port := 49152 + (int(random[0])<<8|int(random[1]))%16384
		if port == 50837 {
			continue
		}
		var err error
		listener, err = net.Listen("tcp4", net.JoinHostPort("127.0.0.1", strconv.Itoa(port)))
		if err == nil {
			break
		}
	}
	if listener == nil {
		t.Fatal("no free isolated port")
	}
	mux := http.NewServeMux()
	server := httptest.NewUnstartedServer(mux)
	server.Listener.Close()
	server.Listener = listener
	server.Start()
	t.Cleanup(server.Close)
	dir := t.TempDir()
	if err := os.Chmod(dir, 0700); err != nil {
		t.Fatal(err)
	}
	token := "bindingtoken000000000000000000000000000000"
	path := filepath.Join(dir, "binding-token")
	if err := os.WriteFile(path, []byte(token), 0600); err != nil {
		t.Fatal(err)
	}
	cfg := StripeIsolatedConfig{Stripe: StripeConfig{Channel: Channel{ID: 1, Provider: "stripe", Merchant: "acct_fixture", Environment: "test", Currency: "USD", Enabled: true}, APIKey: "sk_test_lmm_isolated_00000000000000000000000000000000", APIVersion: "2025-02-24.acacia", SuccessURL: server.URL + "/return/success", CancelURL: server.URL + "/return/cancel"}, TestMode: true, ProviderURL: server.URL, CheckoutOrigin: server.URL, ReturnOrigin: server.URL, SecretDirectory: dir, BindingTokenFile: path}
	return cfg, server, mux
}
func isolatedBinding(t *testing.T, mux *http.ServeMux, cfg StripeIsolatedConfig, mutate func(map[string]any)) {
	t.Helper()
	mux.HandleFunc("/__lmm_isolated_binding", func(w http.ResponseWriter, r *http.Request) {
		if r.Header.Get("Authorization") != "Bearer bindingtoken000000000000000000000000000000" {
			w.WriteHeader(403)
			return
		}
		sum := sha256.Sum256([]byte(cfg.Stripe.APIKey))
		body := map[string]any{"protocol": "lmm-stripe-isolated-v1", "origin": cfg.ProviderURL, "return_origin": cfg.ReturnOrigin, "merchant": cfg.Stripe.Channel.Merchant, "api_key_sha256": hex.EncodeToString(sum[:]), "test_mode": true, "binding_token": "bindingtoken000000000000000000000000000000"}
		if mutate != nil {
			mutate(body)
		}
		json.NewEncoder(w).Encode(body)
	})
}
func TestStripeIsolatedRealHTTPCheckout(t *testing.T) {
	cfg, _, mux := isolatedStripeFixture(t)
	isolatedBinding(t, mux, cfg, nil)
	checkoutURL := cfg.CheckoutOrigin + "/checkout/cs_fixture?session=cs_fixture"
	requests := 0
	mux.HandleFunc("/v1/", func(w http.ResponseWriter, r *http.Request) {
		requests++
		if r.Header.Get("Authorization") != "Bearer "+cfg.Stripe.APIKey || r.Header.Get("X-LMM-Isolated-Binding") != "bindingtoken000000000000000000000000000000" {
			t.Error("unbound provider request")
			w.WriteHeader(403)
			return
		}
		switch r.URL.Path {
		case "/v1/account":
			w.Write([]byte(`{"id":"acct_fixture"}`))
		case "/v1/checkout/sessions":
			if r.Method != "POST" || r.Header.Get("Idempotency-Key") != "isolated_checkout" {
				t.Error("missing request binding")
			}
			r.ParseForm()
			if r.Form.Get("success_url") != cfg.Stripe.SuccessURL {
				t.Error("return mismatch")
			}
			w.Write([]byte(strings.Replace(checkoutObject, "https://checkout.stripe.com/c/pay/test", checkoutURL, 1)))
		default:
			w.WriteHeader(404)
		}
	})
	s, err := NewStripeIsolated(context.Background(), cfg)
	if err != nil {
		t.Fatal(err)
	}
	checkout, err := s.Checkout(context.Background(), bindingOrder(), "isolated_checkout")
	if err != nil || checkout.URL != checkoutURL || requests != 2 {
		t.Fatalf("checkout=%+v requests=%d err=%v", checkout, requests, err)
	}
	for _, bad := range []string{"https://checkout.stripe.com/c/pay/test", cfg.CheckoutOrigin + ".evil/checkout/cs_fixture", cfg.CheckoutOrigin + "/checkout/cs_fixture#evil", cfg.CheckoutOrigin + "/admin", cfg.CheckoutOrigin + "/checkout/%2f%2fevil"} {
		if s.validCheckoutURL(bad) {
			t.Errorf("accepted checkout %s", bad)
		}
	}
	if _, err := NewStripe(cfg.Stripe); err == nil {
		t.Fatal("production constructor accepted loopback returns")
	}
}
func TestStripeIsolatedRejectsConfigurationBeforeIO(t *testing.T) {
	cfg, _, mux := isolatedStripeFixture(t)
	requests := 0
	mux.HandleFunc("/", func(w http.ResponseWriter, r *http.Request) { requests++; w.WriteHeader(500) })
	for name, mutate := range map[string]func(*StripeIsolatedConfig){
		"sharedcredential": func(c *StripeIsolatedConfig) {
			c.ForbiddenCredentials = [][]byte{[]byte("bindingtoken000000000000000000000000000000")}
		},
		"testmode": func(c *StripeIsolatedConfig) { c.TestMode = false }, "live": func(c *StripeIsolatedConfig) { c.Stripe.Channel.Environment = "live" }, "realkey": func(c *StripeIsolatedConfig) { c.Stripe.APIKey = "sk_test_real_secret" }, "webhook": func(c *StripeIsolatedConfig) { c.Stripe.WebhookSecrets = []string{"secret"} }, "othercheckout": func(c *StripeIsolatedConfig) { c.CheckoutOrigin = "http://127.0.0.1:60000" }, "return": func(c *StripeIsolatedConfig) { c.Stripe.SuccessURL = "http://127.0.0.1:60000/success" }, "dir": func(c *StripeIsolatedConfig) { c.SecretDirectory = filepath.Dir(c.SecretDirectory) },
	} {
		t.Run(name, func(t *testing.T) {
			c := cfg
			mutate(&c)
			if _, err := NewStripeIsolated(context.Background(), c); err == nil {
				t.Fatal("accepted unsafe config")
			}
		})
	}
	if requests != 0 {
		t.Fatalf("invalid config performed %d requests", requests)
	}
	for _, origin := range []string{"http://localhost:60000", "http://127.0.0.2:60000", "http://[::ffff:127.0.0.1]:60000", "http://127.0.0.1:80", "http://127.0.0.1:443", "http://127.0.0.1:5432", "http://127.0.0.1:18080", "http://127.0.0.1:18081", "http://127.0.0.1:47399", "http://127.0.0.1:50837", "http://127.0.0.1:060000", "http://user@127.0.0.1:60000", "http://127.0.0.1:60000/", "http://127.0.0.1:60000?x", "http://127.0.0.1:60000#x", "https://127.0.0.1:60000", "http://8.8.8.8:60000"} {
		if _, err := isolatedStripeOrigin(origin); err == nil {
			t.Errorf("accepted origin %s", origin)
		}
	}
	if _, err := isolatedStripeOrigin("http://[::1]:60000"); err != nil {
		t.Error("IPv6 canonical origin rejected")
	}
}
func TestStripeIsolatedBindingAndRedirectDenied(t *testing.T) {
	for _, field := range []string{"protocol", "origin", "return_origin", "merchant", "api_key_sha256", "test_mode", "binding_token", "unknown"} {
		t.Run(field, func(t *testing.T) {
			cfg, _, mux := isolatedStripeFixture(t)
			isolatedBinding(t, mux, cfg, func(body map[string]any) { body[field] = "wrong" })
			if _, err := NewStripeIsolated(context.Background(), cfg); err == nil {
				t.Fatal("accepted binding mismatch")
			}
		})
	}
	t.Run("redirect", func(t *testing.T) {
		cfg, _, mux := isolatedStripeFixture(t)
		followed := false
		mux.HandleFunc("/__lmm_isolated_binding", func(w http.ResponseWriter, r *http.Request) { http.Redirect(w, r, cfg.ProviderURL+"/redirected", 302) })
		mux.HandleFunc("/redirected", func(w http.ResponseWriter, r *http.Request) { followed = true })
		if _, err := NewStripeIsolated(context.Background(), cfg); err == nil || followed {
			t.Fatal("followed redirect")
		}
	})
}
func TestStripeIsolatedPrivateFiles(t *testing.T) {
	cfg, _, mux := isolatedStripeFixture(t)
	isolatedBinding(t, mux, cfg, nil)
	if err := os.Chmod(cfg.BindingTokenFile, 0644); err != nil {
		t.Fatal(err)
	}
	if _, err := NewStripeIsolated(context.Background(), cfg); err == nil {
		t.Fatal("public token accepted")
	}
	os.Chmod(cfg.BindingTokenFile, 0600)
	os.Chmod(cfg.SecretDirectory, 0755)
	if _, err := NewStripeIsolated(context.Background(), cfg); err == nil {
		t.Fatal("public directory accepted")
	}
	os.Chmod(cfg.SecretDirectory, 0700)
	link := filepath.Join(cfg.SecretDirectory, "token-link")
	if err := os.Symlink(cfg.BindingTokenFile, link); err != nil {
		t.Fatal(err)
	}
	cfg.BindingTokenFile = link
	if _, err := NewStripeIsolated(context.Background(), cfg); err == nil {
		t.Fatal("symlink token accepted")
	}
}

func TestStripeIsolatedRealHTTPQueryAndRefund(t *testing.T) {
	cfg, _, mux := isolatedStripeFixture(t)
	isolatedBinding(t, mux, cfg, nil)
	postCount := 0
	queryCount := 0
	mux.HandleFunc("/v1/", func(w http.ResponseWriter, r *http.Request) {
		if r.Header.Get("X-LMM-Isolated-Binding") != "bindingtoken000000000000000000000000000000" {
			t.Error("unbound query")
			w.WriteHeader(403)
			return
		}
		switch r.URL.Path {
		case "/v1/account":
			w.Write([]byte(`{"id":"acct_fixture"}`))
		case "/v1/payment_intents/pi_top_1":
			w.Write([]byte(paymentObject))
		case "/v1/refunds":
			if r.Method == "GET" {
				queryCount++
				if r.URL.Query().Get("payment_intent") != "pi_top_1" || r.URL.Query().Get("limit") != "100" {
					t.Error("lost configured query")
				}
				w.Write([]byte(`{"has_more":false,"data":[]}`))
				return
			}
			postCount++
			r.ParseForm()
			if r.Method != "POST" || r.Header.Get("Idempotency-Key") != "isolated_refund" || r.Form.Get("metadata[lmm_hold_receipt_id]") != "held_by_core" {
				t.Error("lost refund authority binding")
			}
			w.Write([]byte(refundObject))
		default:
			w.WriteHeader(404)
		}
	})
	s, err := NewStripeIsolated(context.Background(), cfg)
	if err != nil {
		t.Fatal(err)
	}
	events, err := s.Lookup(context.Background(), bindingOrder())
	if err != nil || len(events) != 1 || queryCount != 1 {
		t.Fatalf("query events=%d count=%d err=%v", len(events), queryCount, err)
	}
	result, err := s.Refund(context.Background(), bindingOrder(), Refund{ID: "local_refund", AmountMinor: 300, HoldReceipt: "held_by_core"}, "isolated_refund")
	if err != nil || result.ID != "re_fixture" || postCount != 1 {
		t.Fatalf("refund=%+v posts=%d err=%v", result, postCount, err)
	}
}
