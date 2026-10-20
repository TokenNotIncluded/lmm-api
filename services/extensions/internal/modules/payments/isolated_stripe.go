package payments

import (
	"bytes"
	"context"
	"crypto/sha256"
	"crypto/subtle"
	"encoding/hex"
	"encoding/json"
	"net"
	"net/http"
	"net/url"
	"os"
	pathpkg "path"
	"path/filepath"
	"strconv"
	"strings"
	"syscall"
	"time"
)

// StripeIsolatedConfig is an explicit synthetic-provider capability. Production
// composition must continue using NewStripe; this mode never contacts Stripe.
// ProviderURL and CheckoutOrigin are the same canonical numeric HTTP origin.
// The lab owns its listener before construction and keeps its binding token in
// a private directory separate from runtime credentials and webhook secrets.
type StripeIsolatedConfig struct {
	Stripe               StripeConfig
	TestMode             bool
	ProviderURL          string
	CheckoutOrigin       string
	ReturnOrigin         string
	SecretDirectory      string
	BindingTokenFile     string
	ForbiddenCredentials [][]byte
}

type stripeIsolation struct{ origin, token string }

// NewStripeIsolated authenticates an already listening local synthetic lab.
// No webhook secret is accepted here; original callbacks still require Core.
func NewStripeIsolated(ctx context.Context, cfg StripeIsolatedConfig) (*Stripe, error) {
	if !cfg.TestMode || cfg.Stripe.Channel.Environment != "test" || !syntheticStripeKey(cfg.Stripe.APIKey) {
		return nil, ErrInvalid
	}
	origin, err := isolatedStripeOrigin(cfg.ProviderURL)
	if err != nil || cfg.CheckoutOrigin != origin {
		return nil, ErrInvalid
	}
	token, err := isolatedStripeToken(cfg.SecretDirectory, cfg.BindingTokenFile)
	if err != nil {
		return nil, err
	}
	for _, credential := range cfg.ForbiddenCredentials {
		if len(credential) > 0 && (subtle.ConstantTimeCompare(credential, []byte(token)) == 1 || subtle.ConstantTimeCompare(credential, []byte(cfg.Stripe.APIKey)) == 1) {
			return nil, ErrDenied
		}
	}
	if err := validateStripeConfig(cfg.Stripe); err != nil {
		return nil, err
	}
	if !isolatedReturnOrigin(cfg.ReturnOrigin) || !isolatedReturnURL(cfg.Stripe.SuccessURL, cfg.ReturnOrigin) || !isolatedReturnURL(cfg.Stripe.CancelURL, cfg.ReturnOrigin) {
		return nil, ErrInvalid
	}
	s := &Stripe{cfg: cfg.Stripe}
	s.client = isolatedStripeClient(strings.TrimPrefix(origin, "http://"))
	// The challenge is not a payment operation and sends no API key. A random
	// other service on this port cannot establish the configured lab identity.
	raw, err := requestJSON(ctx, s.client, "GET", origin+"/__lmm_isolated_binding", nil, http.Header{"Authorization": {"Bearer " + token}})
	if err != nil {
		return nil, err
	}
	var binding struct {
		Protocol     string `json:"protocol"`
		Origin       string `json:"origin"`
		ReturnOrigin string `json:"return_origin"`
		Merchant     string `json:"merchant"`
		APIKeySHA256 string `json:"api_key_sha256"`
		TestMode     bool   `json:"test_mode"`
		Token        string `json:"binding_token"`
	}
	sum := sha256.Sum256([]byte(cfg.Stripe.APIKey))
	decoder := json.NewDecoder(bytes.NewReader(raw))
	decoder.DisallowUnknownFields()
	if len(raw) > 4096 || jsonObject(raw, &binding) != nil || decoder.Decode(&binding) != nil || binding.ReturnOrigin != cfg.ReturnOrigin || binding.Protocol != "lmm-stripe-isolated-v1" || binding.Origin != origin || binding.Merchant != cfg.Stripe.Channel.Merchant || !binding.TestMode || binding.APIKeySHA256 != hex.EncodeToString(sum[:]) || subtle.ConstantTimeCompare([]byte(binding.Token), []byte(token)) != 1 {
		return nil, ErrDenied
	}
	s.isolated = &stripeIsolation{origin: origin, token: token}
	return s, nil
}

func syntheticStripeKey(key string) bool {
	const prefix = "sk_test_lmm_isolated_"
	return strings.HasPrefix(key, prefix) && isolatedTokenText(strings.TrimPrefix(key, prefix))
}
func isolatedTokenText(s string) bool {
	if len(s) < 32 || len(s) > 128 {
		return false
	}
	for _, r := range s {
		if !(r >= 'a' && r <= 'z' || r >= 'A' && r <= 'Z' || r >= '0' && r <= '9') {
			return false
		}
	}
	return true
}
func isolatedStripeOrigin(raw string) (string, error) {
	u, err := url.Parse(raw)
	if err != nil || u.Scheme != "http" || u.User != nil || u.Opaque != "" || u.Path != "" || u.RawQuery != "" || u.ForceQuery || u.Fragment != "" || u.RawFragment != "" {
		return "", ErrInvalid
	}
	if u.Hostname() != "127.0.0.1" && u.Hostname() != "::1" {
		return "", ErrInvalid
	}
	port, err := strconv.Atoi(u.Port())
	if err != nil || port < 49152 || port > 65535 || port == 50837 || raw != "http://"+net.JoinHostPort(u.Hostname(), strconv.Itoa(port)) {
		return "", ErrInvalid
	}
	return raw, nil
}
func isolatedStripeToken(dir, path string) (string, error) {
	if !filepath.IsAbs(dir) || filepath.Clean(dir) != dir || !filepath.IsAbs(path) || filepath.Clean(path) != path || filepath.Dir(path) != dir {
		return "", ErrInvalid
	}
	real, err := filepath.EvalSymlinks(dir)
	if err != nil || real != dir {
		return "", ErrDenied
	}
	info, err := os.Lstat(dir)
	if err != nil || !info.IsDir() || info.Mode().Perm() != 0700 || !isolatedStripeOwned(info) {
		return "", ErrDenied
	}
	info, err = os.Lstat(path)
	if err != nil || !info.Mode().IsRegular() || info.Mode().Perm() != 0600 || !isolatedStripeOwned(info) {
		return "", ErrDenied
	}
	// O_NOFOLLOW closes the check/open symlink race; compare inode after open.
	fd, err := syscall.Open(path, syscall.O_RDONLY|syscall.O_NOFOLLOW|syscall.O_CLOEXEC, 0)
	if err != nil {
		return "", ErrDenied
	}
	f := os.NewFile(uintptr(fd), path)
	defer f.Close()
	opened, err := f.Stat()
	if err != nil || !os.SameFile(info, opened) || opened.Size() > 128 || opened.Mode().Perm() != 0600 || !isolatedStripeOwned(opened) {
		return "", ErrDenied
	}
	raw, err := boundedBytes(f)
	if err != nil || !isolatedTokenText(string(raw)) {
		return "", ErrInvalid
	}
	return string(raw), nil
}
func isolatedStripeOwned(info os.FileInfo) bool {
	stat, ok := info.Sys().(*syscall.Stat_t)
	return ok && stat.Uid == uint32(os.Geteuid()) && (info.IsDir() || stat.Nlink == 1)
}
func isolatedStripeClient(address string) *http.Client {
	// Fresh transport avoids shared proxies, DNS, TLS changes and global state.
	transport := &http.Transport{MaxIdleConns: 2, IdleConnTimeout: 30 * time.Second, ResponseHeaderTimeout: 5 * time.Second}
	transport.DialContext = func(ctx context.Context, network, target string) (net.Conn, error) {
		if target != address {
			return nil, ErrDenied
		}
		return (&net.Dialer{Timeout: 3 * time.Second}).DialContext(ctx, "tcp", address)
	}
	return &http.Client{Timeout: 8 * time.Second, Transport: transport, CheckRedirect: func(*http.Request, []*http.Request) error { return ErrDenied }}
}
func (s *Stripe) validCheckoutURL(raw string) bool {
	if s.isolated == nil {
		u, err := secureURL(raw)
		return err == nil && u.Hostname() == "checkout.stripe.com"
	}
	u, err := url.Parse(raw)
	return err == nil && u.User == nil && u.Opaque == "" && u.Fragment == "" && u.RawFragment == "" && u.Scheme+"://"+u.Host == s.isolated.origin && strings.HasPrefix(u.Path, "/checkout/") && pathpkg.Clean(u.Path) == u.Path && !strings.Contains(u.Path, "\\") && u.RawPath == ""
}

func isolatedReturnOrigin(raw string) bool {
	if _, err := isolatedStripeOrigin(raw); err == nil {
		return true
	}
	u, err := secureURL(raw)
	return err == nil && u.Path == "" && u.RawQuery == "" && !u.ForceQuery && u.Fragment == "" && u.RawFragment == "" && raw == "https://"+u.Host
}
func isolatedReturnURL(raw, origin string) bool {
	u, err := url.Parse(raw)
	return err == nil && u.User == nil && u.Opaque == "" && u.Scheme+"://"+u.Host == origin && strings.HasPrefix(u.Path, "/") && pathpkg.Clean(u.Path) == u.Path && !strings.Contains(u.Path, "\\") && u.RawPath == "" && u.Fragment == "" && u.RawFragment == "" && !strings.ContainsAny(raw, "\x00\r\n")
}
