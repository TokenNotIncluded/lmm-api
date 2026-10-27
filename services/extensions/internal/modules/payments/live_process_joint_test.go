package payments_test

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/coreclient"
	p "github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/payments"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/payments/rustbridge"
	_ "github.com/lib/pq"
)

type liveProcessFixture struct {
	Socket          string    `json:"socket"`
	Token           string    `json:"token"`
	User            string    `json:"user"`
	Channel         p.Channel `json:"channel"`
	Account         int64     `json:"account"`
	IntentID        int64     `json:"intent_id"`
	PrepareKey      string    `json:"prepare_key"`
	Currency        string    `json:"currency"`
	AmountMinor     int64     `json:"amount_minor"`
	DatabaseURLFile string    `json:"go_database_url_file"`
	DatabaseName    string    `json:"go_database_name"`
	Events          []struct {
		Payload   string `json:"payload"`
		Signature string `json:"signature"`
	} `json:"events"`
}

type liveProcessTransport struct{ fixture liveProcessFixture }

func (tr liveProcessTransport) RoundTrip(r *http.Request) (*http.Response, error) {
	if r.URL.Scheme != "https" || r.URL.Host != "api.stripe.com" || r.URL.User != nil || r.URL.RawQuery != "" || r.URL.Fragment != "" || r.Method != http.MethodGet && r.Method != http.MethodPost || r.Header.Get("Authorization") != "Bearer sk_live_synthetic_process_only" {
		return nil, fmt.Errorf("synthetic provider denied unexpected target")
	}
	f := tr.fixture
	var body []byte
	var err error
	switch r.URL.Path {
	case "/v1/account":
		if r.Method != http.MethodGet {
			return nil, fmt.Errorf("unexpected account method")
		}
		body, err = json.Marshal(map[string]string{"id": f.Channel.Merchant})
	case "/v1/checkout/sessions":
		if r.Method != http.MethodPost {
			return nil, fmt.Errorf("unexpected checkout method")
		}
		var e struct {
			Data struct {
				Object struct {
					ID string `json:"id"`
				} `json:"object"`
			} `json:"data"`
		}
		if json.Unmarshal([]byte(f.Events[0].Payload), &e) != nil {
			return nil, fmt.Errorf("invalid local event fixture")
		}
		order := "top_" + strconv.FormatInt(f.IntentID, 10)
		body, err = json.Marshal(map[string]any{"id": "cs_joint_fixture", "object": "checkout.session", "mode": "payment", "client_reference_id": order, "metadata": map[string]string{"lmm_order_id": order, "lmm_core_intent_id": strconv.FormatInt(f.IntentID, 10), "lmm_merchant": f.Channel.Merchant}, "amount_total": f.AmountMinor, "currency": strings.ToLower(f.Currency), "livemode": true, "payment_intent": e.Data.Object.ID, "url": "https://checkout.stripe.com/c/pay/joint-fixture"})
	default:
		if r.Method == http.MethodGet && strings.HasPrefix(r.URL.Path, "/v1/events/") {
			id := strings.TrimPrefix(r.URL.Path, "/v1/events/")
			for _, event := range f.Events {
				var e struct {
					ID string `json:"id"`
				}
				if json.Unmarshal([]byte(event.Payload), &e) == nil && e.ID == id {
					body = []byte(event.Payload)
					break
				}
			}
		}
		if len(body) == 0 {
			return nil, fmt.Errorf("unexpected local provider API route")
		}
	}
	if err != nil {
		return nil, fmt.Errorf("invalid local provider fixture")
	}
	return &http.Response{StatusCode: 200, Header: http.Header{"Content-Type": {"application/json"}}, Body: io.NopCloser(strings.NewReader(string(body)))}, nil
}

// Only this test executable replaces outbound provider transport. The actual Go
// product has no provider adapter and performs first credit from the durable job.
func TestSyntheticLiveProofAcrossCoreProcess(t *testing.T) {
	path := os.Getenv("PAYMENT_CORE_PROOF_FIXTURE")
	if path == "" {
		t.Skip("synthetic live process fixture not configured; cross-process acceptance NOT RUN")
	}
	phase := os.Getenv("PAYMENT_CORE_PROOF_PHASE")
	if phase != "reject-mode" && phase != "recv" && phase != "settle" {
		t.Fatal("unknown phase")
	}
	info, err := os.Stat(path)
	if err != nil || !info.Mode().IsRegular() || info.Mode().Perm() != 0600 {
		t.Fatal("private fixture required")
	}
	raw, err := os.ReadFile(path)
	if err != nil {
		t.Fatal("cannot read fixture")
	}
	var f liveProcessFixture
	if json.Unmarshal(raw, &f) != nil || f.Channel.Environment != "live" || f.Channel.Provider != "stripe" || f.Channel.Merchant != "acct_live_process_synthetic" || f.AmountMinor != 100 || f.Currency != "USD" || len(f.Events) != 2 || !strings.HasPrefix(f.DatabaseName, "lmm_payments_proof_") {
		t.Fatal("invalid synthetic live fixture")
	}
	ctx, cancel := context.WithTimeout(context.Background(), 45*time.Second)
	defer cancel()
	client, err := coreclient.New(f.Socket, []byte(f.Token))
	if err != nil {
		t.Fatal("core client unavailable")
	}
	defer client.Close()
	bridge, err := rustbridge.New(client)
	if err != nil {
		t.Fatal("bridge unavailable")
	}
	if phase == "reject-mode" {
		var event struct {
			ID   string `json:"id"`
			Data struct {
				Object struct {
					ID string `json:"id"`
				} `json:"object"`
			} `json:"data"`
		}
		if json.Unmarshal([]byte(f.Events[0].Payload), &event) != nil {
			t.Fatal("invalid signed event")
		}
		_, err = bridge.AcceptProof(ctx, f.IntentID, p.Evidence{Provider: "stripe", Merchant: f.Channel.Merchant, Environment: "live", Transaction: event.Data.Object.ID, EventID: event.ID, Payload: []byte(f.Events[0].Payload), Signature: []byte(f.Events[0].Signature), Source: "webhook"})
		if !errors.Is(err, p.ErrDenied) {
			t.Fatal("Core must return permission denial for signed test event on live intent")
		}
		// Also reject a caller-supplied test reference for correctly signed live
		// bytes; neither mode in the raw event nor the reference can override Core.
		if json.Unmarshal([]byte(f.Events[1].Payload), &event) != nil {
			t.Fatal("invalid live event")
		}
		_, err = bridge.AcceptProof(ctx, f.IntentID, p.Evidence{Provider: "stripe", Merchant: f.Channel.Merchant, Environment: "test", Transaction: event.Data.Object.ID, EventID: event.ID, Payload: []byte(f.Events[1].Payload), Signature: []byte(f.Events[1].Signature), Source: "webhook"})
		if !errors.Is(err, p.ErrDenied) {
			t.Fatal("Core must deny test reference on live intent")
		}
		badSignature := strings.Replace(f.Events[1].Signature, "v1=", "v1=00", 1)
		_, err = bridge.AcceptProof(ctx, f.IntentID, p.Evidence{Provider: "stripe", Merchant: f.Channel.Merchant, Environment: "live", Transaction: event.Data.Object.ID, EventID: event.ID, Payload: []byte(f.Events[1].Payload), Signature: []byte(badSignature), Source: "webhook"})
		if !errors.Is(err, p.ErrDenied) {
			t.Fatal("Core must deny malformed live signature")
		}
		return
	}
	info, err = os.Stat(f.DatabaseURLFile)
	if err != nil || !info.Mode().IsRegular() || info.Mode().Perm() != 0600 {
		t.Fatal("private Go DSN file required")
	}
	dsn, err := os.ReadFile(f.DatabaseURLFile)
	if err != nil {
		t.Fatal("cannot read private DSN")
	}
	db, err := sql.Open("postgres", strings.TrimSpace(string(dsn)))
	if err != nil {
		t.Fatal("cannot open isolated Go PG")
	}
	defer db.Close()
	store, err := p.NewPostgres(ctx, db, f.DatabaseName)
	if err != nil {
		t.Fatal("storage guard rejected fixture")
	}
	stripe, err := p.NewStripe(p.StripeConfig{Channel: f.Channel, APIKey: "sk_live_synthetic_process_only", APIVersion: "synthetic-process-version", SuccessURL: "https://synthetic.example.invalid/paid", CancelURL: "https://synthetic.example.invalid/canceled"})
	if err != nil {
		t.Fatal("invalid synthetic live adapter")
	}
	p.ReplaceStripeTransportForTest(stripe, liveProcessTransport{fixture: f})
	service, err := p.New(store, bridge, stripe)
	if err != nil {
		t.Fatal("cannot construct service")
	}
	callback := httptest.NewServer(service.WebhookHandler())
	defer callback.Close()
	httpClient := callback.Client()
	httpClient.Timeout = 12 * time.Second
	orderID := "top_" + strconv.FormatInt(f.IntentID, 10)
	if phase == "recv" {
		order, err := service.Create(ctx, f.User, p.PrepareRequest{Key: f.PrepareKey, AccountID: f.Account, ChannelID: f.Channel.ID, Currency: f.Currency, AmountMinor: f.AmountMinor})
		if err != nil || order.ID != orderID {
			t.Fatal("prepare did not match frozen live intent")
		}
	}
	end := len(f.Events)
	if phase == "recv" {
		end = 1
	}
	for _, event := range f.Events[:end] {
		request, err := http.NewRequestWithContext(ctx, http.MethodPost, callback.URL+"/callbacks/"+strconv.FormatInt(f.Channel.ID, 10), strings.NewReader(event.Payload))
		if err != nil {
			t.Fatal("request failed")
		}
		request.Header.Set("Stripe-Signature", event.Signature)
		response, err := httpClient.Do(request)
		if err != nil {
			t.Fatal("local callback failed")
		}
		_, _ = io.CopyN(io.Discard, response.Body, 4096)
		response.Body.Close()
		if response.StatusCode != http.StatusOK {
			t.Fatalf("callback status %d", response.StatusCode)
		}
	}
	if err := service.Drain(ctx, 100); err != nil {
		t.Fatal("drain failed")
	}
	order, err := store.Get(ctx, orderID)
	if err != nil {
		t.Fatal("order unavailable")
	}
	if phase == "recv" && order.Credited() {
		t.Fatal("accepting proof claimed money")
	}
	if phase == "settle" {
		if !order.Credited() {
			t.Fatal("real Go main must credit before replay")
		}
		receipt := order.CreditReceipt
		if err := service.Work(ctx, 100); err != nil {
			t.Fatal("replay failed")
		}
		order, err = store.Get(ctx, orderID)
		if err != nil || !order.Credited() || order.CreditReceipt != receipt {
			t.Fatal("replay changed receipt")
		}
	}
}
