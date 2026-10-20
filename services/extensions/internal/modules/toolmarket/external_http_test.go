// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later
package toolmarket

import (
	"context"
	"errors"
	"net"
	"net/http"
	"net/http/httptest"
	"sync"
	"sync/atomic"
	"testing"
	"time"
)

func TestExternalHTTPURLPolicy(t *testing.T) {
	for _, raw := range []string{"https://example.com/search?q=hello", "https://8.8.8.8/search"} {
		if ValidateExternalHTTPURL(raw, false) != nil {
			t.Fatal(raw)
		}
	}
	for _, raw := range []string{"http://example.com", "https://localhost", "https://127.0.0.1", "https://169.254.169.254", "https://192.0.2.1", "https://user:pass@example.com", "https://example.com/#fragment", "https://example.com:444/", "https://[::ffff:127.0.0.1]/"} {
		if ValidateExternalHTTPURL(raw, false) == nil {
			t.Fatal(raw)
		}
	}
	for _, raw := range []string{"http://localhost", "http://example.com", "http://192.168.1.1", "https://127.0.0.1"} {
		if ValidateExternalHTTPURL(raw, true) == nil {
			t.Fatal(raw)
		}
	}
	if ValidateExternalHTTPURL("http://127.0.0.1:1234/search?q=x", true) != nil {
		t.Fatal("literal loopback rejected")
	}
}
func TestExternalHTTPClientNoRedirectProxyOrReuse(t *testing.T) {
	var calls, target atomic.Int32
	destination := httptest.NewServer(http.HandlerFunc(func(http.ResponseWriter, *http.Request) { target.Add(1) }))
	defer destination.Close()
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { calls.Add(1); http.Redirect(w, r, destination.URL, 302) }))
	defer server.Close()
	t.Setenv("HTTP_PROXY", "http://127.0.0.1:1")
	client, err := NewExternalHTTPClient(true)
	if err != nil {
		t.Fatal(err)
	}
	response, err := client.Get(server.URL + "/?q=test")
	if err != nil {
		t.Fatal(err)
	}
	response.Body.Close()
	if response.StatusCode != 302 || calls.Load() != 1 || target.Load() != 0 {
		t.Fatal(response.StatusCode, calls.Load(), target.Load())
	}
	transport := client.Transport.(externalHTTPTransport).base
	if transport.Proxy != nil || !transport.DisableKeepAlives || transport.ForceAttemptHTTP2 || len(transport.TLSNextProto) != 0 {
		t.Fatal("unsafe transport options")
	}
	production, _ := NewExternalHTTPClient(false)
	if _, err := production.Get(server.URL); err == nil {
		t.Fatal("public client admitted loopback")
	}
}
func TestExternalDialAdmissionBoundAndCancellation(t *testing.T) {
	var active, maximum atomic.Int32
	release := make(chan struct{})
	entered := make(chan struct{}, 4)
	dial := boundedExternalDial(func(context.Context, string, string) (net.Conn, error) {
		n := active.Add(1)
		for old := maximum.Load(); n > old && !maximum.CompareAndSwap(old, n); old = maximum.Load() {
		}
		entered <- struct{}{}
		<-release
		active.Add(-1)
		return nil, errors.New("synthetic dial failure")
	})
	var workers sync.WaitGroup
	for i := 0; i < 4; i++ {
		workers.Add(1)
		go func() { defer workers.Done(); dial(context.Background(), "tcp", "unused") }()
	}
	for i := 0; i < 4; i++ {
		select {
		case <-entered:
		case <-time.After(time.Second):
			t.Fatal("dial did not enter")
		}
	}
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	if _, err := dial(ctx, "tcp", "unused"); !errors.Is(err, context.Canceled) {
		t.Fatal(err)
	}
	close(release)
	workers.Wait()
	if maximum.Load() != 4 || active.Load() != 0 {
		t.Fatal(maximum.Load(), active.Load())
	}
}
