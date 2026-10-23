// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package rustbridge

import (
	"context"
	"encoding/json"
	"errors"
	market "github.com/TokenNotIncluded/lmm-api/services/extensions/internal/modules/toolmarket"
	"net"
	"net/http"
	"path/filepath"
	"strings"
	"sync"
	"testing"
)

func TestFixedWitnessRequiresIndependentConfigAndReviewedReadiness(t *testing.T) {
	ordinary := filepath.Join(t.TempDir(), "ordinary.sock")
	token := []byte(strings.Repeat("o", 40))
	a, e := New(ordinary, token)
	if e != nil {
		t.Fatal(e)
	}
	defer a.Close()
	if e = a.WithFixedWitness(filepath.Join(t.TempDir(), "purpose.sock"), token); !errors.Is(e, market.ErrInvalid) {
		t.Fatal("ordinary bearer reused", e)
	}
	if e = a.WithFixedWitness("relative", []byte(strings.Repeat("w", 40))); !errors.Is(e, market.ErrInvalid) {
		t.Fatal("relative socket", e)
	}
	// Core routes purposes by distinct bearer, including on the same UDS. The
	// transport/client remains independently constructed and cannot reuse a token.
	if e = a.WithFixedWitness(ordinary, []byte(strings.Repeat("w", 40))); e != nil {
		t.Fatal(e)
	}
	if a.client == a.fixedWitness.client {
		t.Fatal("purpose reused ordinary HTTP client")
	}
	if a.FixedWitnessAvailable() {
		t.Fatal("configuration invented installer review")
	}
	if _, e = a.FinishLegacyCredits(context.Background(), market.LegacyCreditFinish{Success: true, ProofKind: market.LegacyProofFixedWitness, RawResult: "e30=", ResultDigest: strings.Repeat("a", 64)}); !errors.Is(e, market.ErrUnavailable) {
		t.Fatal("unreviewed purpose called", e)
	}
}
func TestFixedWitnessRoutingNeverUsesOrdinaryBearerOrSerializedRole(t *testing.T) {
	base := t.TempDir()
	ordinaryPath := filepath.Join(base, "ordinary.sock")
	purposePath := filepath.Join(base, "purpose.sock")
	ordinaryToken := strings.Repeat("o", 40)
	purposeToken := strings.Repeat("w", 40)
	var mu sync.Mutex
	ordinaryCalls, purposeCalls := 0, 0
	serve := func(path, token string, purpose bool) {
		listener, e := net.Listen("unix", path)
		if e != nil {
			t.Fatal(e)
		}
		server := &http.Server{Handler: http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			mu.Lock()
			defer mu.Unlock()
			if r.Header.Get("Authorization") != "Bearer "+token || r.Header.Get("X-Lmm-Protocol") != "2" || r.Header.Get("X-Lmm-User-Credential") != "" {
				t.Error("purpose credential routing changed")
			}
			expected := "/internal/tool-market/v2/calls/settle"
			if purpose {
				expected = "/internal/tool-market/v2/fixed-witness/settle"
				if strings.HasSuffix(r.URL.Path, "/release") {
					expected = "/internal/tool-market/v2/fixed-witness/release"
				}
				purposeCalls++
			} else {
				ordinaryCalls++
			}
			if r.URL.Path != expected {
				t.Error("wrong endpoint", r.URL.Path)
			}
			var body map[string]json.RawMessage
			if json.NewDecoder(r.Body).Decode(&body) != nil {
				t.Error("invalid body")
			}
			if len(body) != 5 {
				t.Error("unexpected proof/body fields", body)
			}
			for _, field := range []string{"role", "proof_kind", "purpose", "payer", "price_mode", "held"} {
				if _, ok := body[field]; ok {
					t.Error("untrusted body selected authority", field)
				}
			}
			_ = json.NewEncoder(w).Encode(market.LegacyCreditCall{ID: "call", State: "settled"})
		})}
		done := make(chan error, 1)
		go func() { done <- server.Serve(listener) }()
		t.Cleanup(func() { _ = server.Close(); <-done })
	}
	serve(ordinaryPath, ordinaryToken, false)
	serve(purposePath, purposeToken, true)
	a, e := New(ordinaryPath, []byte(ordinaryToken))
	if e != nil {
		t.Fatal(e)
	}
	defer a.Close()
	if e = a.WithFixedWitness(purposePath, []byte(purposeToken)); e != nil {
		t.Fatal(e)
	}
	// Synthetic transport fixture only: real readiness must come from Core's
	// installer-reviewed purpose capability, independently tested with CorePG.
	a.fixedWitnessReviewed = true
	fixed := market.LegacyCreditFinish{Success: true, ProofKind: market.LegacyProofFixedWitness, RawResult: "e30=", ResultDigest: strings.Repeat("a", 64)}
	if _, e = a.FinishLegacyCredits(context.Background(), fixed); e != nil {
		t.Fatal(e)
	}
	failed := fixed
	failed.Success = false
	if _, e = a.FinishLegacyCredits(context.Background(), failed); e != nil {
		t.Fatal("purpose failed release", e)
	}
	free := fixed
	free.ProofKind = market.LegacyProofOrdinary
	if _, e = a.FinishLegacyCredits(context.Background(), free); e != nil {
		t.Fatal(e)
	}
	metered := free
	metered.Usage = &market.LegacyCreditUsage{Source: "tool_reported", Quantities: map[string]int64{"images": 1}}
	if _, e = a.FinishLegacyCredits(context.Background(), metered); e != nil {
		t.Fatal(e)
	}
	bad := fixed
	bad.Usage = metered.Usage
	if _, e = a.FinishLegacyCredits(context.Background(), bad); !errors.Is(e, market.ErrInvalid) {
		t.Fatal("metered used fixed witness", e)
	}
	mu.Lock()
	defer mu.Unlock()
	if ordinaryCalls != 2 || purposeCalls != 2 {
		t.Fatal("wrong authority port", ordinaryCalls, purposeCalls)
	}
}

func TestFixedWitnessReadyAndPerQuoteAdmissionRequireActualCoreACK(t *testing.T) {
	socket := filepath.Join(t.TempDir(), "core.sock")
	listener, e := net.Listen("unix", socket)
	if e != nil {
		t.Fatal(e)
	}
	var mu sync.Mutex
	purpose := "core_host_fixed_success"
	features := []string{"admit", "settle", "release"}
	admits := 0
	server := &http.Server{Handler: http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		mu.Lock()
		defer mu.Unlock()
		if r.Header.Get("Authorization") != "Bearer "+strings.Repeat("w", 40) {
			w.WriteHeader(401)
			return
		}
		var body map[string]json.RawMessage
		if json.NewDecoder(r.Body).Decode(&body) != nil {
			w.WriteHeader(400)
			return
		}
		switch r.URL.Path {
		case "/internal/tool-market/v2/fixed-witness/capabilities":
			if len(body) != 0 {
				t.Error("capability body claimed authority", body)
			}
			_ = json.NewEncoder(w).Encode(map[string]any{"protocol_major": 2, "purpose": purpose, "role": "reviewed-role", "features": features})
		case "/internal/tool-market/v2/fixed-witness/admit":
			admits++
			if len(body) != 1 {
				t.Error("admit body selected role", body)
			}
			var quote string
			_ = json.Unmarshal(body["quote_id"], &quote)
			if quote != "approved-quote" {
				w.WriteHeader(403)
				return
			}
			_ = json.NewEncoder(w).Encode(map[string]any{"quote_id": quote, "role": "reviewed-role", "proof_mode": "core_host_fixed_success", "ready": true})
		default:
			t.Error("unexpected effectful request", r.URL.Path)
			w.WriteHeader(403)
		}
	})}
	done := make(chan error, 1)
	go func() { done <- server.Serve(listener) }()
	defer func() { _ = server.Close(); <-done }()
	a, e := New(socket, []byte(strings.Repeat("o", 40)))
	if e != nil {
		t.Fatal(e)
	}
	defer a.Close()
	if e = a.WithFixedWitness(socket, []byte(strings.Repeat("w", 40))); e != nil {
		t.Fatal(e)
	}
	if e = a.FixedWitnessReady(context.Background()); e != nil || !a.FixedWitnessAvailable() {
		t.Fatal("reviewed capability unavailable", e)
	}
	if e = a.AdmitFixedWitness(context.Background(), "approved-quote"); e != nil {
		t.Fatal(e)
	}
	if e = a.AdmitFixedWitness(context.Background(), "different-role-quote"); !errors.Is(e, market.ErrDenied) {
		t.Fatal("quote not admitted was accepted", e)
	}
	mu.Lock()
	features = []string{"settle", "release"}
	mu.Unlock()
	if e = a.FixedWitnessReady(context.Background()); e == nil || a.FixedWitnessAvailable() {
		t.Fatal("missing per-quote admission capability accepted")
	}
	mu.Lock()
	features = []string{"admit", "settle", "release"}
	purpose = "claimed_in_body"
	mu.Unlock()
	if e = a.FixedWitnessReady(context.Background()); e == nil || a.FixedWitnessAvailable() {
		t.Fatal("wrong purpose accepted")
	}
	mu.Lock()
	defer mu.Unlock()
	if admits != 2 {
		t.Fatal(admits)
	}
}
