// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later
package toolmarket

import (
	"context"
	"crypto/tls"
	"errors"
	"net"
	"net/http"
	"net/netip"
	"net/url"
	"strings"
	"time"
)

// DNS and connection setup share a process-wide bounded admission gate. The
// existing network dialer validates every DNS answer and dials checked IPs.
var externalDialSlots = make(chan struct{}, 4)

func boundedExternalDial(dial func(context.Context, string, string) (net.Conn, error)) func(context.Context, string, string) (net.Conn, error) {
	return func(ctx context.Context, kind, address string) (net.Conn, error) {
		select {
		case externalDialSlots <- struct{}{}:
			defer func() { <-externalDialSlots }()
		case <-ctx.Done():
			return nil, ctx.Err()
		}
		return dial(ctx, kind, address)
	}
}

type externalHTTPTransport struct {
	base      *http.Transport
	localHTTP bool
}

func (t externalHTTPTransport) RoundTrip(request *http.Request) (*http.Response, error) {
	if request == nil || validateExternalURL(request.URL, t.localHTTP) != nil {
		return nil, ErrDenied
	}
	return t.base.RoundTrip(request)
}
func validateExternalURL(u *url.URL, local bool) error {
	if u == nil || u.Host == "" || u.User != nil || u.Fragment != "" || u.Opaque != "" || len(u.String()) > 32768 || strings.ContainsAny(u.String(), "\r\n\\") {
		return ErrDenied
	}
	host := strings.ToLower(u.Hostname())
	ip, iperr := netip.ParseAddr(host)
	if local {
		if u.Scheme != "http" || iperr != nil || !ip.Unmap().IsLoopback() {
			return ErrDenied
		}
		return nil
	}
	if u.Scheme != "https" || (u.Port() != "" && u.Port() != "443") || host == "localhost" || strings.HasSuffix(host, ".localhost") || strings.HasSuffix(host, ".local") || strings.Contains(host, "%") || (iperr == nil && !publicIP(ip)) {
		return ErrDenied
	}
	return nil
}

// NewExternalHTTPClient reuses ToolMarket's all-public DNS validation and IP
// pinning. LocalHTTP admits only literal loopback HTTP test endpoints. It never
// consults proxy environment variables, follows redirects, or reuses connections
// that net/http could automatically retry after an ambiguous network failure.
func NewExternalHTTPClient(localHTTP bool) (*http.Client, error) {
	n := newNetwork()
	tr, ok := n.client.Transport.(*http.Transport)
	if !ok {
		return nil, errors.New("external HTTP transport unavailable")
	}
	tr = tr.Clone()
	if localHTTP {
		dialer := &net.Dialer{Timeout: 5 * time.Second}
		tr.DialContext = func(ctx context.Context, kind, address string) (net.Conn, error) {
			host, _, err := net.SplitHostPort(address)
			ip, e := netip.ParseAddr(host)
			if err != nil || e != nil || !ip.Unmap().IsLoopback() {
				return nil, ErrDenied
			}
			return dialer.DialContext(ctx, kind, address)
		}
	}
	tr.DialContext = boundedExternalDial(tr.DialContext)
	tr.DisableKeepAlives = true
	tr.ForceAttemptHTTP2 = false
	tr.TLSNextProto = map[string]func(string, *tls.Conn) http.RoundTripper{}
	tr.MaxResponseHeaderBytes = 16 * 1024
	return &http.Client{Transport: externalHTTPTransport{tr, localHTTP}, Timeout: 10 * time.Second, CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}, nil
}

// ValidateExternalHTTPURL checks host policy without DNS or network access.
func ValidateExternalHTTPURL(raw string, localHTTP bool) error {
	u, err := url.Parse(raw)
	if err != nil {
		return ErrDenied
	}
	return validateExternalURL(u, localHTTP)
}
