package coreclient

import (
	"bytes"
	"context"
	"encoding/json"
	"io"
	"strings"

	pb "github.com/TokenNotIncluded/lmm-api/services/extensions/internal/corepb"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
)

const AssistantBusinessFactsFeature = "assistant.business_facts.v1"
const maxAssistantBusinessFacts = 1 << 20

func (c *AssistantClient) BusinessFactsReady(ctx context.Context) error {
	return c.assistantFeatures(ctx, AssistantBusinessFactsFeature)
}
func decodeBusinessFacts(raw []byte, out any) bool {
	if _, ok := boundedAssistantObject(raw, maxAssistantBusinessFacts); !ok {
		return false
	}
	d := json.NewDecoder(bytes.NewReader(raw))
	d.DisallowUnknownFields()
	if d.Decode(out) != nil {
		return false
	}
	var extra any
	return d.Decode(&extra) == io.EOF
}
func validBusinessFacts(raw []byte, kind pb.AssistantBusinessFactKind, user int64) bool {
	switch kind {
	case pb.AssistantBusinessFactKind_ASSISTANT_BUSINESS_FACT_KIND_AVAILABLE_MODELS:
		var r struct {
			Actor     int64           `json:"actor_user_id"`
			Models    []string        `json:"exact_model_ids"`
			Developer *bool           `json:"developer_access_granted"`
			Locked    *bool           `json:"locked"`
			Preview   *bool           `json:"public_preview"`
			Groups    []string        `json:"key_group_options"`
			Catalogue json.RawMessage `json:"catalogue"`
		}
		if !decodeBusinessFacts(raw, &r) || r.Actor != user || r.Developer == nil || r.Locked == nil || r.Preview == nil || len(r.Models) > 4096 || len(r.Groups) > 128 || len(r.Catalogue) == 0 {
			return false
		}
		for _, list := range [][]string{r.Models, r.Groups} {
			seen := map[string]bool{}
			for _, id := range list {
				if !assistantLabel(id, 256) || seen[id] {
					return false
				}
				seen[id] = true
			}
		}
		if !bytes.Equal(r.Catalogue, []byte("null")) {
			if _, ok := boundedAssistantObject(r.Catalogue, maxAssistantBusinessFacts); !ok {
				return false
			}
		}
		return true
	case pb.AssistantBusinessFactKind_ASSISTANT_BUSINESS_FACT_KIND_SUPPORT_ELIGIBILITY:
		var r struct {
			OK       bool   `json:"ok"`
			Scope    string `json:"scope"`
			Actor    int64  `json:"actor_user_id"`
			Eligible bool   `json:"eligible"`
			Payments int64  `json:"successful_external_payments"`
			Amount   string `json:"unrefunded_amount_minor"`
			Currency string `json:"currency"`
			Evidence string `json:"evidence"`
			Excluded bool   `json:"wallet_balance_is_not_qualification"`
			Checked  int64  `json:"checked_at_unix"`
			ID       string `json:"projection_id"`
			Kind     string `json:"projection_kind"`
		}
		if !decodeBusinessFacts(raw, &r) || !r.OK || r.Scope != "self" || r.Actor != user || r.Payments < 0 || r.Eligible != (r.Payments > 0) || r.Currency != "USD" || r.Evidence != "core verified external payment receipts" || !r.Excluded || r.Checked <= 0 || !assistantDigest(r.ID, "support-observation-v1:") || r.Kind != "authenticated_read_observation_not_financial_receipt" || len(r.Amount) == 0 || len(r.Amount) > 40 {
			return false
		}
		for _, ch := range r.Amount {
			if ch < '0' || ch > '9' {
				return false
			}
		}
		return (strings.TrimLeft(r.Amount, "0") != "") == r.Eligible
	default:
		return false
	}
}

// Kind is a closed read contract. JSON can never select an operation, payer,
// model dispatch, SQL or authority. Both credentials remain server metadata.
func (c *AssistantClient) GetAssistantBusinessFacts(ctx context.Context, user, kind string) (json.RawMessage, error) {
	var selection pb.AssistantBusinessFactKind
	switch kind {
	case "available_models":
		selection = pb.AssistantBusinessFactKind_ASSISTANT_BUSINESS_FACT_KIND_AVAILABLE_MODELS
	case "support_eligibility":
		selection = pb.AssistantBusinessFactKind_ASSISTANT_BUSINESS_FACT_KIND_SUPPORT_ELIGIBILITY
	default:
		return nil, status.Error(codes.InvalidArgument, "invalid business facts kind")
	}
	if !validUserCredential(user) {
		return nil, status.Error(codes.Unauthenticated, "invalid user credential")
	}
	if err := c.BusinessFactsReady(ctx); err != nil {
		return nil, err
	}
	actor, err := c.control.Authorize(ctx, user)
	if err != nil {
		return nil, err
	}
	if !assistantMessage(actor) || actor.GetUserId() <= 0 || actor.GetCredentialKind() != pb.CredentialKind_CREDENTIAL_KIND_SESSION || actor.GetOwner().GetKind() != pb.AccountKind_ACCOUNT_KIND_PERSONAL || actor.GetOwner().GetId() != actor.GetUserId() {
		return nil, status.Error(codes.PermissionDenied, "personal Session required")
	}
	r, err := pb.NewCoreAssistantModelClient(c.rpc.connection).GetAssistantBusinessFacts(c.rpc.context(ctx, user), &pb.GetAssistantBusinessFactsRequest{Kind: selection})
	if err != nil {
		return nil, err
	}
	if !assistantMessage(r) || !validBusinessFacts(r.GetFactsJson(), selection, actor.GetUserId()) {
		return nil, status.Error(codes.DataLoss, "invalid business facts response")
	}
	return append(json.RawMessage(nil), r.GetFactsJson()...), nil
}
