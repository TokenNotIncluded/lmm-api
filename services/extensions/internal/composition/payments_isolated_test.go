package composition

import (
	"context"
	"crypto/rand"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"net"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"testing"
)

func isolatedPaymentDocument(t *testing.T) (string, string, string, string, *int) {
	t.Helper()
	var listener net.Listener
	for i := 0; i < 1024; i++ {
		var entropy [2]byte
		if _, err := rand.Read(entropy[:]); err != nil {
			t.Fatal(err)
		}
		port := 49152 + int(uint16(entropy[0])<<8|uint16(entropy[1]))%16384
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
		t.Fatal("no isolated listener")
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
	apiKey := "sk_test_lmm_isolated_00000000000000000000000000000000"
	token := "loaderbinding0000000000000000000000000000"
	keyPath := filepath.Join(dir, "api-key")
	tokenPath := filepath.Join(dir, "binding-token")
	if err := os.WriteFile(keyPath, []byte(apiKey), 0600); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(tokenPath, []byte(token), 0600); err != nil {
		t.Fatal(err)
	}
	hostOrigin := "http://127.0.0.1:61501"
	if hostOrigin == server.URL {
		hostOrigin = "http://127.0.0.1:61502"
	}
	requests := new(int)
	mux.HandleFunc("/__lmm_isolated_binding", func(w http.ResponseWriter, r *http.Request) {
		*requests++
		if r.Method != "GET" || r.Header.Get("Authorization") != "Bearer "+token {
			w.WriteHeader(403)
			return
		}
		sum := sha256.Sum256([]byte(apiKey))
		json.NewEncoder(w).Encode(map[string]any{"protocol": "lmm-stripe-isolated-v1", "origin": server.URL, "return_origin": hostOrigin, "merchant": "acct_isolated_loader", "api_key_sha256": hex.EncodeToString(sum[:]), "test_mode": true, "binding_token": token})
	})
	doc := fmt.Sprintf("version=1\nprovider='stripe-isolated-test'\n[[channels]]\nid=1\nmerchant='acct_isolated_loader'\nenabled=true\napi_key_file='%s'\napi_version='2025-02-24.acacia'\nsuccess_url='%s/return/success?status=ok'\ncancel_url='%s/return/cancel'\nisolated_test_mode=true\nisolated_provider_url='%s'\nisolated_checkout_origin='%s'\nisolated_return_origin='%s'\nisolated_secret_directory='%s'\nisolated_binding_token_file='%s'\n", keyPath, hostOrigin, hostOrigin, server.URL, server.URL, hostOrigin, dir, tokenPath)
	return doc, hostOrigin, apiKey, token, requests
}
func TestPaymentIsolatedProviderLoaderRealHTTP(t *testing.T) {
	doc, origin, _, _, requests := isolatedPaymentDocument(t)
	path := paymentPrivate(t, "isolated.toml", doc)
	credentials := [][]byte{[]byte("host0000000000000000000000000000000000"), []byte("core0000000000000000000000000000000000")}
	adapters, err := loadPaymentAdaptersForHost(context.Background(), path, origin, true, credentials...)
	if err != nil || len(adapters) != 1 || *requests != 1 || adapters[0].Channel().Environment != "test" {
		t.Fatalf("adapters=%d requests=%d err=%v", len(adapters), *requests, err)
	}
}
func TestPaymentIsolatedProviderLoaderRejectsUnsafeConfiguration(t *testing.T) {
	doc, origin, apiKey, token, requests := isolatedPaymentDocument(t)
	host := []byte("host0000000000000000000000000000000000")
	for _, tc := range []struct {
		name, document, hostOrigin string
		local                      bool
		credentials                [][]byte
	}{
		{"no_host", doc, "", true, [][]byte{host}},
		{"unbound_host", doc, "http://127.0.0.1:61503", true, [][]byte{host}},
		{"no_profile", doc, origin, false, [][]byte{host}},
		{"no_credentials", doc, origin, true, nil},
		{"empty_host_credential", doc, origin, true, [][]byte{nil}},
		{"reused_binding", doc, origin, true, [][]byte{host, []byte(token)}},
		{"reused_key", doc, origin, true, [][]byte{host, []byte(apiKey)}},
		{"disabled_mode", strings.Replace(doc, "isolated_test_mode=true", "isolated_test_mode=false", 1), origin, true, [][]byte{host}},
		{"production_provider", strings.Replace(doc, "stripe-isolated-test", "stripe-test", 1), origin, true, [][]byte{host}},
		{"unknown_field", doc + "isolated_ignored_field=true\n", origin, true, [][]byte{host}},
		{"external_provider", strings.ReplaceAll(doc, "isolated_provider_url='http://127.0.0.1:", "isolated_provider_url='http://8.8.8.8:"), origin, true, [][]byte{host}},
	} {
		t.Run(tc.name, func(t *testing.T) {
			if _, err := loadPaymentAdaptersForHost(context.Background(), paymentPrivate(t, "invalid.toml", tc.document), tc.hostOrigin, tc.local, tc.credentials...); err == nil {
				t.Fatal("unsafe config accepted")
			}
		})
	}
	if *requests != 0 {
		t.Fatalf("unsafe config performed %d handshake requests", *requests)
	}
}
func TestPaymentProductionRejectsEvenEmptyIsolatedFields(t *testing.T) {
	key := paymentPrivate(t, "key", "sk_test_synthetic_host_key_0000001")
	base := fmt.Sprintf("version=1\nprovider='stripe-test'\n[[channels]]\nid=1\nmerchant='acct_production_loader'\nenabled=true\napi_key_file='%s'\napi_version='2025-02-24.acacia'\nsuccess_url='https://example.test/success'\ncancel_url='https://example.test/cancel'\n", key)
	for _, field := range []string{"isolated_test_mode=false", "isolated_provider_url=''", "isolated_checkout_origin=''", "isolated_return_origin=''", "isolated_secret_directory=''", "isolated_binding_token_file=''"} {
		if _, err := loadPaymentAdapters(paymentPrivate(t, "production.toml", base+field+"\n")); err == nil {
			t.Fatalf("ignored isolated field %s", field)
		}
	}
}

func TestPaymentIsolatedRejectsAlreadyLoadedMarketCredentials(t *testing.T) {
	doc, origin, apiKey, bindingToken, requests := isolatedPaymentDocument(t)
	path := paymentPrivate(t, "isolated.toml", doc)
	host := []byte("host0000000000000000000000000000000000")
	for _, purpose := range []string{"market", "encryption", "witness", "wallet", "bounty", "image"} {
		for _, reuse := range []string{apiKey, bindingToken} {
			t.Run(purpose+"_"+reuse[:5], func(t *testing.T) {
				runtime := &Runtime{}
				loaded := [][]byte{[]byte("market00000000000000000000000000000000"), []byte("encryption0000000000000000000000000000"), []byte("witness0000000000000000000000000000000"), []byte("wallet00000000000000000000000000000000"), []byte("bounty00000000000000000000000000000000"), []byte("image000000000000000000000000000000000")}
				for index, name := range []string{"market", "encryption", "witness", "wallet", "bounty", "image"} {
					if name == purpose {
						loaded[index] = []byte(reuse)
					}
				}
				runtime.retainPaymentIsolationCredentials([]string{"toolmarket", "payments"}, loaded...)
				credentials := append([][]byte{host}, runtime.paymentIsolationCredentials...)
				if _, err := loadPaymentAdaptersForHost(context.Background(), path, origin, true, credentials...); err == nil {
					t.Fatal("already loaded role reused as payment lab credential")
				}
			})
		}
	}
	if *requests != 0 {
		t.Fatalf("reused roles performed %d provider requests", *requests)
	}
	runtime := &Runtime{}
	runtime.retainPaymentIsolationCredentials([]string{"toolmarket"}, []byte(bindingToken))
	if len(runtime.paymentIsolationCredentials) != 0 {
		t.Fatal("retained startup secrets without payments selected")
	}
}
