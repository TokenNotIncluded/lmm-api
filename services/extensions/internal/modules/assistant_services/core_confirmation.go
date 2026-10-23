package assistant_services

import (
	"context"
	"encoding/hex"
	"encoding/json"
	"github.com/TokenNotIncluded/api.lmm.best/extensions/internal/coreclient"
	pb "github.com/TokenNotIncluded/api.lmm.best/extensions/internal/corepb"
	"github.com/TokenNotIncluded/api.lmm.best/extensions/internal/modules/assistant"
	"github.com/TokenNotIncluded/api.lmm.best/extensions/internal/modules/assistant/access"
	"strings"
)

var _ BusinessFactsClient = (*coreclient.AssistantClient)(nil)
var _ ConfirmationAuthority = (*CoreConfirmationAuthority)(nil)

type CoreConfirmationAuthority struct {
	client *coreclient.AssistantClient
	auth   access.Authority
}

func NewCoreConfirmationAuthority(client *coreclient.AssistantClient, auth access.Authority) (*CoreConfirmationAuthority, error) {
	if client == nil || access.Missing(auth) {
		return nil, access.ErrInvalid
	}
	return &CoreConfirmationAuthority{client, auth}, nil
}
func (c *CoreConfirmationAuthority) Ready(ctx context.Context) error {
	return c.client.ConfirmationReady(ctx)
}
func (c *CoreConfirmationAuthority) Prepare(ctx context.Context, credential, operation, tool, sourceSHA string, draft json.RawMessage) (Challenge, error) {
	var object map[string]json.RawMessage
	if len(draft) > 16384 || access.Decode(draft, &object) != nil {
		return Challenge{}, access.ErrInvalid
	}
	r, err := c.client.PrepareAssistantToolConfirmation(ctx, credential, &pb.PrepareAssistantToolConfirmationRequest{ToolOperationId: operation, NormalizedDraftJson: append([]byte(nil), draft...)})
	if err != nil {
		return Challenge{}, access.Translate(err)
	}
	if !validCoreConfirmation(r, operation) || r.GetToolName() != tool || hex.EncodeToString(r.GetSourceArgumentsSha256()) != sourceSHA {
		return Challenge{}, access.ErrUnavailable
	}
	return Challenge{ID: r.GetChallengeId(), URL: r.GetBrowserPath(), PayloadSHA256: hex.EncodeToString(r.GetNormalizedDraftSha256()), ExpiresAt: r.GetExpiresAtUnix()}, nil
}
func (c *CoreConfirmationAuthority) Verify(ctx context.Context, credential, challenge, operation, draftSHA string) (BrowserProof, error) {
	hash, err := hex.DecodeString(draftSHA)
	if err != nil || len(hash) != 32 {
		return BrowserProof{}, access.ErrInvalid
	}
	r, err := c.client.GetAssistantToolConfirmationReceipt(ctx, credential, &pb.GetAssistantToolConfirmationReceiptRequest{ToolOperationId: operation, NormalizedDraftSha256: hash})
	if err != nil {
		return BrowserProof{}, access.Translate(err)
	}
	if !validCoreConfirmation(r, operation) || r.GetChallengeId() != challenge || r.GetState() != "approved" || hex.EncodeToString(r.GetNormalizedDraftSha256()) != draftSHA {
		return BrowserProof{}, access.ErrForbidden
	}
	p, err := c.auth.Check(ctx, credential, access.Account{}, access.Read)
	if err != nil {
		return BrowserProof{}, err
	}
	if p.UserID <= 0 || p.Account != (access.Account{Kind: "personal", ID: p.UserID}) {
		return BrowserProof{}, access.ErrForbidden
	}
	return BrowserProof{Confirmed: true, UserID: p.UserID, OperationID: operation, PayloadSHA256: draftSHA, ReceiptID: r.GetChallengeId(), ExpiresAt: r.GetExpiresAtUnix()}, nil
}
func validCoreConfirmation(r *pb.AssistantToolConfirmation, operation string) bool {
	if r == nil || r.GetToolOperationId() != operation || r.GetToolVersion() != assistant.FixedMainToolVersion || r.GetExecuted() || len(r.GetSourceArgumentsSha256()) != 32 || len(r.GetNormalizedDraftSha256()) != 32 || r.GetChallengeId() == "" || len(r.GetChallengeId()) > 256 || r.GetExpiresAtUnix() <= 0 {
		return false
	}
	return r.GetBrowserPath() == "/api/user/assistant/confirmations/"+r.GetChallengeId() && !strings.ContainsAny(r.GetChallengeId(), "/\\\x00\r\n")
}
