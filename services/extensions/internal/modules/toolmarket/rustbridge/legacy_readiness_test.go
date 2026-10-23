// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package rustbridge

import (
	"context"
	"encoding/json"
	market "github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/toolmarket"
	"net"
	"net/http"
	"path/filepath"
	"strings"
	"sync"
	"testing"
)

func readinessReceipt() market.LegacyCapabilities {
	return market.LegacyCapabilities{ProtocolMajor: 2, Unit: "credit_500k_usd", Features: strings.Fields("quotes quotes.approve access.consent-create access.consent-revoke access.budget-set access.token-create access.token-revoke access.token-validate access.client-disconnect access.consent-list access.budget-list access.token-list calls.reserve calls.dispatch calls.settle calls.unknown calls.release calls.query config.read config.update services.lifecycle actors.personal reports.create reports.review reports.list income.list meta.read meta.owner-read meta.owner-set meta.authorize meta.client-budget meta.tool-budget meta.usage meta.calls meta.oauth-clients actors.oauth oauth.enabled")}
}
func TestLegacyReadyRequiresActualV2AckAndClearsFailedProbe(t *testing.T) {
	socket := filepath.Join(t.TempDir(), "r")
	l, e := net.Listen("unix", socket)
	if e != nil {
		t.Fatal(e)
	}
	host := strings.Repeat("h", 43)
	var mu sync.Mutex
	mode := "good"
	server := &http.Server{Handler: http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method != "POST" || r.URL.Path != "/internal/tool-market/v2/capabilities" || r.Header.Get("Authorization") != "Bearer "+host || r.Header.Get("X-Lmm-Protocol") != "2" || r.Header.Get("X-Lmm-User-Credential") != "" {
			t.Error("readiness host-only protocol boundary changed")
			w.WriteHeader(403)
			return
		}
		var in map[string]json.RawMessage
		if json.NewDecoder(r.Body).Decode(&in) != nil || len(in) != 0 {
			t.Error("readiness took user identity/config input")
			w.WriteHeader(400)
			return
		}
		mu.Lock()
		current := mode
		mu.Unlock()
		ack := readinessReceipt()
		switch current {
		case "unit":
			ack.Unit = "usd_micros"
		case "protocol":
			ack.ProtocolMajor = 1
		case "duplicate":
			ack.Features = append(ack.Features, "future", "future")
		case "missing":
			ack.Features = ack.Features[1:]
		case "no-oauth":
			ack.Features = ack.Features[:len(ack.Features)-2]
		case "omitted":
			w.Write([]byte(`{"protocol_major":2,"features":[]}`))
			return
		case "future":
			ack.Features = append(ack.Features, "future.feature")
		}
		json.NewEncoder(w).Encode(ack)
	})}
	done := make(chan error, 1)
	go func() { done <- server.Serve(l) }()
	t.Cleanup(func() { server.Close(); <-done })
	a, e := New(socket, []byte(host))
	if e != nil {
		t.Fatal(e)
	}
	defer a.Close()
	ctx := context.Background()
	if a.LegacyReadiness().ProtocolMajor != 0 {
		t.Fatal("constructor pretended readiness")
	}
	if e = a.LegacyReady(ctx, true); e != nil {
		t.Fatal(e)
	}
	snapshot := a.LegacyReadiness()
	snapshot.Features[0] = "tampered"
	if a.LegacyReadiness().Features[0] == "tampered" {
		t.Fatal("readonly ACK mutated by caller")
	}
	for _, bad := range []string{"unit", "protocol", "duplicate", "missing", "omitted"} {
		mu.Lock()
		mode = bad
		mu.Unlock()
		if a.LegacyReady(ctx, false) == nil {
			t.Fatal("accepted", bad)
		}
		if a.LegacyReadiness().ProtocolMajor != 0 {
			t.Fatal("failed probe retained old ACK", bad)
		}
		mu.Lock()
		mode = "good"
		mu.Unlock()
		if e = a.LegacyReady(ctx, false); e != nil {
			t.Fatal(e)
		}
	}
	mu.Lock()
	mode = "no-oauth"
	mu.Unlock()
	if e = a.LegacyReady(ctx, false); e != nil {
		t.Fatal("personal-only real Core denied", e)
	}
	if a.LegacyReady(ctx, true) == nil {
		t.Fatal("OAuth configuration accepted missing profile ACK")
	}
	mu.Lock()
	mode = "future"
	mu.Unlock()
	if e = a.LegacyReady(ctx, true); e != nil {
		t.Fatal("added feature rejected", e)
	}
}
func TestLegacyReadyOldInFlightAckCannotReviveNewFailedProbe(t *testing.T) {
	socket := filepath.Join(t.TempDir(), "g")
	l, e := net.Listen("unix", socket)
	if e != nil {
		t.Fatal(e)
	}
	host := strings.Repeat("h", 43)
	entered := make(chan struct{})
	release := make(chan struct{})
	var mu sync.Mutex
	calls := 0
	server := &http.Server{Handler: http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		mu.Lock()
		calls++
		call := calls
		mu.Unlock()
		if call == 1 {
			close(entered)
			<-release
			json.NewEncoder(w).Encode(readinessReceipt())
			return
		}
		w.WriteHeader(503)
	})}
	done := make(chan error, 1)
	go func() { done <- server.Serve(l) }()
	t.Cleanup(func() { server.Close(); <-done })
	a, e := New(socket, []byte(host))
	if e != nil {
		t.Fatal(e)
	}
	defer a.Close()
	old := make(chan error, 1)
	go func() { old <- a.LegacyReady(t.Context(), false) }()
	<-entered
	if a.LegacyReady(t.Context(), false) == nil {
		t.Fatal("failed newer probe accepted")
	}
	close(release)
	if <-old == nil {
		t.Fatal("old ACK won after newer failure")
	}
	if a.LegacyReadiness().ProtocolMajor != 0 {
		t.Fatal("old ACK revived readiness")
	}
}
