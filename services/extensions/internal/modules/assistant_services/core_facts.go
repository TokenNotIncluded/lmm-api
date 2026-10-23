package assistant_services

import (
	"bytes"
	"context"
	"encoding/json"
	"github.com/TokenNotIncluded/api.lmm.best/extensions/internal/modules/assistant/access"
	"io"
	"strings"
	"time"
)

// BusinessFactsClient is implemented by the original-session private Core
// client. Kind is a closed host enum, never a model supplied query or SQL.
type BusinessFactsClient interface {
	BusinessFactsReady(context.Context) error
	GetAssistantBusinessFacts(context.Context, string, string) (json.RawMessage, error)
}
type CoreBusinessFacts struct {
	client BusinessFactsClient
	auth   access.Authority
}

func NewCoreBusinessFacts(client BusinessFactsClient, auth access.Authority) (*CoreBusinessFacts, error) {
	if access.Missing(client) || access.Missing(auth) {
		return nil, access.ErrInvalid
	}
	return &CoreBusinessFacts{client, auth}, nil
}
func (c *CoreBusinessFacts) Ready(ctx context.Context) error { return c.client.BusinessFactsReady(ctx) }
func (c *CoreBusinessFacts) actor(ctx context.Context, credential string) (access.Principal, error) {
	p, err := c.auth.Check(ctx, credential, access.Account{}, access.Read)
	if err != nil {
		return p, err
	}
	if p.UserID <= 0 || p.Account != (access.Account{Kind: "personal", ID: p.UserID}) {
		return p, access.ErrForbidden
	}
	return p, nil
}
func (c *CoreBusinessFacts) AvailableModels(ctx context.Context, credential string) (ModelCatalogSnapshot, error) {
	p, err := c.actor(ctx, credential)
	if err != nil {
		return ModelCatalogSnapshot{}, err
	}
	raw, err := c.client.GetAssistantBusinessFacts(ctx, credential, "available_models")
	if err != nil {
		return ModelCatalogSnapshot{}, access.Translate(err)
	}
	var r struct {
		ActorUserID int64           `json:"actor_user_id"`
		IDs         []string        `json:"exact_model_ids"`
		Developer   bool            `json:"developer_access_granted"`
		Locked      bool            `json:"locked"`
		Preview     bool            `json:"public_preview"`
		Groups      []string        `json:"key_group_options"`
		Catalogue   json.RawMessage `json:"catalogue"`
	}
	if decodeFacts(raw, &r) != nil || r.ActorUserID != p.UserID || len(r.IDs) > 4096 || len(r.Groups) > 128 {
		return ModelCatalogSnapshot{}, access.ErrUnavailable
	}
	groups := make([]PublicKeyGroup, 0, len(r.Groups))
	seen := map[string]bool{}
	for _, id := range r.Groups {
		if id == "" || len(id) > 128 || seen[id] {
			return ModelCatalogSnapshot{}, access.ErrUnavailable
		}
		seen[id] = true
		groups = append(groups, PublicKeyGroup{ID: id})
	}
	return ModelCatalogSnapshot{ActorUserID: r.ActorUserID, DeveloperAccessGranted: r.Developer, AccountModelAccessLocked: r.Locked, PublicPreview: r.Preview, ModelIDs: append([]string(nil), r.IDs...), KeyGroupOptions: groups}, nil
}
func decodeFacts(raw []byte, out any) error {
	if len(raw) == 0 || len(raw) > 1<<20 {
		return access.ErrUnavailable
	}
	d := json.NewDecoder(bytes.NewReader(raw))
	d.DisallowUnknownFields()
	if d.Decode(out) != nil {
		return access.ErrUnavailable
	}
	var trailing any
	if d.Decode(&trailing) != io.EOF {
		return access.ErrUnavailable
	}
	return nil
}

// SupportEligibility records an authenticated Core read observation. Its digest
// is explicitly not a financial receipt, permission grant, or wallet balance.
func (c *CoreBusinessFacts) SupportEligibility(ctx context.Context, credential string) (Eligibility, error) {
	p, err := c.actor(ctx, credential)
	if err != nil {
		return Eligibility{}, err
	}
	raw, err := c.client.GetAssistantBusinessFacts(ctx, credential, "support_eligibility")
	if err != nil {
		return Eligibility{}, access.Translate(err)
	}
	var r struct {
		OK             bool   `json:"ok"`
		Scope          string `json:"scope"`
		Actor          int64  `json:"actor_user_id"`
		Eligible       bool   `json:"eligible"`
		Payments       int64  `json:"successful_external_payments"`
		Amount         string `json:"unrefunded_amount_minor"`
		Currency       string `json:"currency"`
		Evidence       string `json:"evidence"`
		WalletExcluded bool   `json:"wallet_balance_is_not_qualification"`
		Checked        int64  `json:"checked_at_unix"`
		ID             string `json:"projection_id"`
		Kind           string `json:"projection_kind"`
	}
	if decodeFacts(raw, &r) != nil || !r.OK || r.Scope != "self" || r.Actor != p.UserID || r.Payments < 0 || r.Eligible != (r.Payments > 0) || r.Currency != "USD" || r.Evidence != "core verified external payment receipts" || !r.WalletExcluded || r.Kind != "authenticated_read_observation_not_financial_receipt" || !strings.HasPrefix(r.ID, "support-observation-v1:") || !validSHA(strings.TrimPrefix(r.ID, "support-observation-v1:")) || r.Checked < time.Now().Unix()-30 || r.Checked > time.Now().Unix()+5 {
		return Eligibility{}, access.ErrUnavailable
	}
	// The decimal is evidence metadata, never money accepted from Go. Bound its
	// representation without lossy float conversion or any payment API call.
	if len(r.Amount) == 0 || len(r.Amount) > 40 {
		return Eligibility{}, access.ErrUnavailable
	}
	for _, digit := range r.Amount {
		if digit < '0' || digit > '9' {
			return Eligibility{}, access.ErrUnavailable
		}
	}
	if positive := strings.TrimLeft(r.Amount, "0") != ""; positive != r.Eligible {
		return Eligibility{}, access.ErrUnavailable
	}
	return Eligibility{Eligible: r.Eligible, ReceiptID: r.ID, CheckedAt: r.Checked}, nil
}
