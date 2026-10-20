package assistant_services

import (
	"context"
	"encoding/json"
	"github.com/TokenNotIncluded/api.lmm.best/extensions/internal/modules/assistant/access"
	"strings"
	"testing"
	"time"
)

type syntheticFactsClient struct {
	raw              json.RawMessage
	credential, kind string
}

func (c *syntheticFactsClient) BusinessFactsReady(context.Context) error { return nil }
func (c *syntheticFactsClient) GetAssistantBusinessFacts(_ context.Context, credential, kind string) (json.RawMessage, error) {
	c.credential, c.kind = credential, kind
	return c.raw, nil
}
func TestCoreBusinessFactsProjectionBinding(t *testing.T) {
	auth := &guideAuth{principal: access.Principal{UserID: 7, Account: access.Account{Kind: "personal", ID: 7}}}
	client := &syntheticFactsClient{raw: json.RawMessage(`{"actor_user_id":7,"exact_model_ids":["serving/model"],"developer_access_granted":false,"locked":false,"public_preview":true,"key_group_options":["actual-group"],"catalogue":null}`)}
	facts, err := NewCoreBusinessFacts(client, auth)
	if err != nil {
		t.Fatal(err)
	}
	got, err := facts.AvailableModels(context.Background(), "native-session")
	if err != nil || got.ActorUserID != 7 || len(got.ModelIDs) != 1 || got.KeyGroupOptions[0].Description != "" || client.credential != "native-session" || client.kind != "available_models" {
		t.Fatal(got, err)
	}
	client.raw = json.RawMessage(strings.Replace(string(client.raw), `"actor_user_id":7`, `"actor_user_id":8`, 1))
	if _, err = facts.AvailableModels(context.Background(), "native-session"); err == nil {
		t.Fatal("foreign actor accepted")
	}
}
func TestCoreSupportObservationIsNotWalletOrMoneyReceipt(t *testing.T) {
	auth := &guideAuth{principal: access.Principal{UserID: 7, Account: access.Account{Kind: "personal", ID: 7}}}
	client := &syntheticFactsClient{}
	facts, _ := NewCoreBusinessFacts(client, auth)
	payload := map[string]any{"ok": true, "scope": "self", "actor_user_id": 7, "eligible": true, "successful_external_payments": 1, "unrefunded_amount_minor": "100", "currency": "USD", "evidence": "core verified external payment receipts", "wallet_balance_is_not_qualification": true, "checked_at_unix": time.Now().Unix(), "projection_id": "support-observation-v1:" + strings.Repeat("a", 64), "projection_kind": "authenticated_read_observation_not_financial_receipt"}
	client.raw, _ = json.Marshal(payload)
	got, err := facts.SupportEligibility(context.Background(), "native-session")
	if err != nil || !got.Eligible || got.ReceiptID != payload["projection_id"] || client.kind != "support_eligibility" {
		t.Fatal(got, err)
	}
	for _, field := range []string{"actor_user_id", "projection_kind", "wallet_balance_is_not_qualification", "checked_at_unix", "unrefunded_amount_minor"} {
		t.Run(field, func(t *testing.T) {
			bad := map[string]any{}
			for k, v := range payload {
				bad[k] = v
			}
			switch field {
			case "actor_user_id":
				bad[field] = 8
			case "projection_kind":
				bad[field] = "wallet_balance"
			case "wallet_balance_is_not_qualification":
				bad[field] = false
			case "checked_at_unix":
				bad[field] = time.Now().Unix() - 60
			case "unrefunded_amount_minor":
				bad[field] = "0"
			}
			client.raw, _ = json.Marshal(bad)
			if _, err := facts.SupportEligibility(context.Background(), "native-session"); err == nil {
				t.Fatal("invalid Core observation accepted")
			}
		})
	}
}
