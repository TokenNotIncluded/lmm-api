package payments_test

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/TokenNotIncluded/api.lmm.best/extensions/internal/coreclient"
	p "github.com/TokenNotIncluded/api.lmm.best/extensions/internal/modules/payments"
	"github.com/TokenNotIncluded/api.lmm.best/extensions/internal/modules/payments/rustbridge"
)

// This executable is a real Go module worker process, not lmm-extensions main.
// Only the Stripe transport destination changes; the production parser, storage,
// state machine and Rust bridge are used unchanged. No webhook secret enters Go.
type refundProcessTransport struct {
	base   *url.URL
	client *http.Client
}

func (tr refundProcessTransport) RoundTrip(request *http.Request) (*http.Response, error) {
	clone := request.Clone(request.Context())
	target := *request.URL
	target.Scheme, target.Host = tr.base.Scheme, tr.base.Host
	clone.URL = &target
	clone.Host = tr.base.Host
	return tr.client.Do(clone)
}

type refundProcessFixture struct {
	proofJointFixture
	ProviderURL  string `json:"provider_url"`
	MarkerDir    string `json:"marker_dir"`
	EvidenceFile string `json:"evidence_file"`
}
type refundSignedEvent struct {
	Payload   string `json:"payload"`
	Signature string `json:"signature"`
}
type refundRecoveryEvidence struct {
	Created       refundSignedEvent `json:"created"`
	Failed        refundSignedEvent `json:"failed"`
	LateSuccess   refundSignedEvent `json:"late_success"`
	ConflictingID refundSignedEvent `json:"conflicting_id"`
}

func refundPrivateJSON(t *testing.T, path string, value any) {
	t.Helper()
	raw, err := json.Marshal(value)
	if err != nil {
		t.Fatal("cannot encode private receipt")
	}
	file, err := os.OpenFile(path, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0600)
	if err != nil {
		t.Fatal("private receipt already exists or cannot be created")
	}
	if _, err = file.Write(raw); err != nil {
		t.Fatal("cannot write receipt")
	}
	if err = file.Sync(); err != nil {
		t.Fatal("cannot sync receipt")
	}
	if err = file.Close(); err != nil {
		t.Fatal("cannot close receipt")
	}
}
func refundEvidence(f refundProcessFixture, event refundSignedEvent) p.Evidence {
	var parsed struct {
		ID string `json:"id"`
	}
	if json.Unmarshal([]byte(event.Payload), &parsed) != nil {
		panic("invalid synthetic event")
	}
	return p.Evidence{Provider: f.Channel.Provider, Merchant: f.Channel.Merchant, Environment: f.Channel.Environment, Transaction: "pi_" + strconv.FormatInt(f.IntentID, 10), EventID: parsed.ID, Payload: []byte(event.Payload), Signature: []byte(event.Signature), Source: "webhook"}
}

func TestRefundProcessWorkerFixture(t *testing.T) {
	path, phase := os.Getenv("LMM_REFUND_PROCESS_FIXTURE"), os.Getenv("LMM_REFUND_PROCESS_PHASE")
	if path == "" {
		t.Skip("isolated refund process orchestrator required; NOT RUN")
	}
	if phase != "dispatch" && phase != "recover" {
		t.Fatal("invalid refund process phase")
	}
	info, err := os.Stat(path)
	if err != nil || info.Mode().Perm()&0077 != 0 {
		t.Fatal("fixture must be private")
	}
	raw, err := os.ReadFile(path)
	if err != nil {
		t.Fatal("cannot read fixture")
	}
	var f refundProcessFixture
	if json.Unmarshal(raw, &f) != nil || f.IntentID <= 0 || f.Account <= 0 || f.Channel.Environment != "test" || len(f.Events) == 0 {
		t.Fatal("invalid synthetic fixture")
	}
	u, err := url.Parse(f.ProviderURL)
	if err != nil || u.Scheme != "http" || u.Hostname() != "127.0.0.1" || u.User != nil {
		t.Fatal("provider must be owned loopback TCP fixture")
	}
	info, err = os.Stat(f.DatabaseURLFile)
	if err != nil || info.Mode().Perm()&0077 != 0 {
		t.Fatal("Go database URL must be private")
	}
	dsn, err := os.ReadFile(f.DatabaseURLFile)
	if err != nil {
		t.Fatal("cannot read isolated Go database reference")
	}
	ctx, cancel := context.WithTimeout(context.Background(), 150*time.Second)
	defer cancel()
	db, err := sql.Open("postgres", strings.TrimSpace(string(dsn)))
	if err != nil {
		t.Fatal("cannot open isolated Go database")
	}
	defer db.Close()
	db.SetMaxOpenConns(4)
	store, err := p.NewPostgres(ctx, db, f.DatabaseName)
	if err != nil {
		t.Fatal("Go isolation guard rejected fixture")
	}
	client, err := coreclient.New(f.Socket, []byte(f.Token))
	if err != nil {
		t.Fatal("cannot connect real Core")
	}
	defer client.Close()
	bridge, err := rustbridge.New(client)
	if err != nil {
		t.Fatal(err)
	}
	stripe, err := p.NewStripe(p.StripeConfig{Channel: f.Channel, APIKey: "sk_test_refund_synthetic_only", APIVersion: "refund-synthetic-version", SuccessURL: "https://refund.example.invalid/paid", CancelURL: "https://refund.example.invalid/cancel"})
	if err != nil {
		t.Fatal(err)
	}
	p.ReplaceStripeTransportForTest(stripe, refundProcessTransport{base: u, client: &http.Client{Transport: &http.Transport{Proxy: nil}, Timeout: 12 * time.Second}})
	service, err := p.New(store, bridge, stripe)
	if err != nil {
		t.Fatal(err)
	}
	callback := httptest.NewServer(service.WebhookHandler())
	defer callback.Close()
	deliver := func(event refundSignedEvent) {
		req, err := http.NewRequestWithContext(ctx, "POST", callback.URL+"/callbacks/1", strings.NewReader(event.Payload))
		if err != nil {
			t.Fatal(err)
		}
		req.Header.Set("Stripe-Signature", event.Signature)
		response, err := callback.Client().Do(req)
		if err != nil {
			t.Fatal("callback loopback failed")
		}
		_, _ = io.Copy(io.Discard, response.Body)
		_ = response.Body.Close()
		if response.StatusCode != http.StatusOK {
			t.Fatalf("Core-backed callback status %d", response.StatusCode)
		}
	}
	orderID := "top_" + strconv.FormatInt(f.IntentID, 10)
	if phase == "dispatch" {
		_, err = service.Create(ctx, f.User, p.PrepareRequest{Key: f.PrepareKey, AccountID: f.Account, ChannelID: 1, Currency: "USD", AmountMinor: 100})
		if err != nil {
			t.Fatal("original Core prepare mismatch")
		}
		deliver(refundSignedEvent{Payload: f.Events[0].Payload, Signature: f.Events[0].Signature})
		for i := 0; i < 3; i++ {
			if err = service.Work(ctx, 100); err != nil {
				t.Fatal(err)
			}
		}
		order, err := store.Get(ctx, orderID)
		if err != nil || !order.Credited() {
			t.Fatal("real Core credit did not settle")
		}
		refund, err := service.RequestRefund(ctx, f.User, orderID, "refund_real_os_0001", 40)
		if err != nil || refund.HoldReceipt == "" {
			t.Fatal("real Core reservation missing")
		}
		refundPrivateJSON(t, filepath.Join(f.MarkerDir, "held.json"), map[string]any{"pid": os.Getpid(), "refund": refund})
		// The owned TCP provider pauses after receiving the POST. The orchestrator
		// must kill this OS process; no object reconstruction is used as a restart.
		if err = service.Work(ctx, 100); err != nil {
			t.Fatal(err)
		}
		t.Fatal("orchestrator failed to stop the worker at provider response loss")
	}
	var original p.Refund
	var submitKey string
	var unknownJob p.Job
	for {
		if err = service.Work(ctx, 100); err != nil {
			t.Fatal(err)
		}
		order, err := store.Get(ctx, orderID)
		if err != nil {
			t.Fatal(err)
		}
		for key, job := range order.Jobs {
			if job.Kind == "refund_submit" && job.State == "review" {
				original = order.Refunds[job.RefundID]
				submitKey = key
				unknownJob = job
			}
		}
		if submitKey != "" {
			break
		}
		select {
		case <-ctx.Done():
			t.Fatal("original lease did not recover")
		case <-time.After(100 * time.Millisecond):
		}
	}
	if unknownJob.Attempts != 2 || unknownJob.ErrorCode != p.ErrUnsupported.Error() {
		t.Fatal("UNKNOWN did not stop the second provider attempt before IO")
	}
	if original.Dispatch != p.DispatchUnknown || original.State == "released" || original.DispatchReceipt == "" {
		t.Fatal("unknown dispatch or Core registration was lost")
	}
	refundPrivateJSON(t, filepath.Join(f.MarkerDir, "unknown.json"), map[string]any{"pid": os.Getpid(), "refund": original, "submit_key": submitKey, "job": unknownJob})
	var evidence refundRecoveryEvidence
	for {
		raw, err = os.ReadFile(f.EvidenceFile)
		if err == nil {
			if json.Unmarshal(raw, &evidence) != nil {
				t.Fatal("invalid immutable signed evidence")
			}
			break
		}
		if !os.IsNotExist(err) {
			t.Fatal("cannot read evidence gate")
		}
		select {
		case <-ctx.Done():
			t.Fatal("evidence gate deadline")
		case <-time.After(50 * time.Millisecond):
		}
	}
	for i := 0; i < 2; i++ {
		deliver(evidence.Created)
		deliver(evidence.Failed)
		for j := 0; j < 3; j++ {
			if err = service.Work(ctx, 100); err != nil {
				t.Fatal(err)
			}
		}
	}
	order, err := store.Get(ctx, orderID)
	if err != nil || order.Refunds[original.ID].State != "released" {
		t.Fatal("bound failure did not release real Core hold")
	}
	// A fresh valid signature does not make a changed payload for an existing
	// event ID valid, nor can later success consume a previously released hold.
	conflicting := refundEvidence(f, evidence.ConflictingID)
	if _, err = bridge.AcceptProof(ctx, f.IntentID, conflicting); !errors.Is(err, p.ErrConflict) {
		t.Fatalf("conflicting signed event ID accepted: %v", err)
	}
	late := refundEvidence(f, evidence.LateSuccess)
	proof, err := bridge.AcceptProof(ctx, f.IntentID, late)
	if err != nil {
		t.Fatal("late authentic proof not retained")
	}
	late.AcceptedProofID = proof.ID
	if _, err = bridge.Commit(ctx, f.IntentID, "commit_after_release_os01", original.HoldReceipt, late); !errors.Is(err, p.ErrConflict) {
		t.Fatalf("success after release consumed reservation: %v", err)
	}
	refundPrivateJSON(t, filepath.Join(f.MarkerDir, "settled.json"), map[string]any{"pid": os.Getpid(), "refund": order.Refunds[original.ID], "submit_key": submitKey, "duplicate_events_checked": true, "conflicting_event_id_denied": true, "success_after_release_denied": true})
	// Keep the actual executed image alive for parent FD/PID attestation.
	for {
		select {
		case <-ctx.Done():
			t.Fatal("orchestrator did not reap settled worker")
		case <-time.After(50 * time.Millisecond):
		}
	}
}
