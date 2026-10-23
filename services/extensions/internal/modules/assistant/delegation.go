package assistant

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"strconv"
	"strings"
	"time"
	"unicode"
	"unicode/utf8"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
)

// ErrDelegationUnknown requires inspection of the original issue/operation. A
// caller must retain its binding, not create a new turn, grant, round or payer.
var (
	ErrDelegationUnknown     = errors.New("delegation_unknown")
	ErrDelegationBudget      = errors.New("budget_exceeded")
	ErrDelegationUnavailable = errors.New("delegation_unavailable")
)

type DelegationIssue struct {
	Version        int    `json:"version"`
	ConversationID string `json:"conversation_id"`
	TurnID         string `json:"turn_id"`
}

type DelegationGrant struct {
	Version        int    `json:"version"`
	ID             string `json:"delegation_id"`
	ConversationID string `json:"conversation_id"`
	TurnID         string `json:"turn_id"`
	ExpiresAt      int64  `json:"expires_at_unix"`
	MaxModelCalls  int    `json:"max_model_calls"`
}

type DelegationBinding struct {
	Version        int    `json:"version"`
	ID             string `json:"delegation_id"`
	ConversationID string `json:"conversation_id"`
	TurnID         string `json:"turn_id"`
	Round          int    `json:"round"`
	OperationKey   string `json:"operation_key"`
	PayloadSHA256  string `json:"payload_sha256"`
}

type DelegationInvocation struct {
	DelegationBinding
	RequestJSON string `json:"request_json"`
}

type DelegationOutcome struct {
	DelegationBinding
	Status             string      `json:"status"`
	CheckpointSequence int64       `json:"checkpoint_sequence"`
	ChargedCredits     *string     `json:"charged_credits"`
	Reply              *ModelReply `json:"reply"`
}

// DelegationBridge is A07's frozen v1 typed port. Implementers must authenticate
// the host AND original live Core Session and strictly decode bounded wire DTOs.
// No production transport/issuer is provided here. This is not Model.Next: an
// interactive token cannot be passed to the API-key model path as a substitute.
type DelegationBridge interface {
	Issue(context.Context, string, DelegationIssue) (DelegationGrant, error)
	Invoke(context.Context, string, DelegationInvocation) (DelegationOutcome, error)
	Status(context.Context, string, DelegationBinding) (DelegationOutcome, error)
}

type DelegationClient struct {
	bridge DelegationBridge
	now    func() time.Time
}

func NewDelegationClient(bridge DelegationBridge) (*DelegationClient, error) {
	if access.Missing(bridge) {
		return nil, access.ErrUnavailable
	}
	return &DelegationClient{bridge: bridge, now: time.Now}, nil
}

func delegationLabel(value string, max int) bool {
	return value != "" && len(value) <= max && utf8.ValidString(value) && !strings.ContainsFunc(value, func(r rune) bool {
		return unicode.IsSpace(r) || unicode.IsControl(r)
	})
}

func lowerDigest(value string) bool {
	if len(value) != 64 {
		return false
	}
	for _, c := range value {
		if !(c >= '0' && c <= '9' || c >= 'a' && c <= 'f') {
			return false
		}
	}
	return true
}

func delegationID(value string) bool {
	return strings.HasPrefix(value, "lmm_ad_") && lowerDigest(strings.TrimPrefix(value, "lmm_ad_"))
}

func (c *DelegationClient) Issue(ctx context.Context, credential, conversationID, turnID string) (DelegationGrant, error) {
	if !delegationLabel(credential, 256) || !delegationLabel(conversationID, 128) || !delegationLabel(turnID, 124) {
		return DelegationGrant{}, access.ErrInvalid
	}
	request := DelegationIssue{Version: 1, ConversationID: conversationID, TurnID: turnID}
	grant, err := c.bridge.Issue(ctx, credential, request)
	if err != nil {
		// An ACK may have been lost after durable issuance. This client never
		// automatically retries Issue or invents a replacement turn.
		return DelegationGrant{}, delegationBridgeError(err)
	}
	if !c.validGrant(grant) || grant.ConversationID != conversationID || grant.TurnID != turnID {
		return DelegationGrant{}, access.ErrUnavailable
	}
	return grant, nil
}

func (c *DelegationClient) validGrant(grant DelegationGrant) bool {
	now := c.now().Unix()
	return grant.Version == 1 && delegationID(grant.ID) && delegationLabel(grant.ConversationID, 128) &&
		delegationLabel(grant.TurnID, 124) && grant.ExpiresAt > now && grant.ExpiresAt-now <= 900 &&
		grant.MaxModelCalls >= 1 && grant.MaxModelCalls <= 4
}

// Prepare serializes the typed request exactly once. Persist the returned
// invocation/binding BEFORE Invoke; recovery uses Status on that same binding.
// Core independently verifies the owner, preceding settled round, exact tool
// scope, payload, original funding policy and cumulative integer budget.
func (c *DelegationClient) Prepare(grant DelegationGrant, round int, request ModelRequest) (DelegationInvocation, error) {
	if !c.validGrant(grant) || round < 0 || round >= grant.MaxModelCalls || !utf8.ValidString(request.Input) || len(request.Input) == 0 || len(request.Input) > 8192 {
		return DelegationInvocation{}, access.ErrInvalid
	}
	encoded, err := json.Marshal(request)
	if err != nil || len(encoded) > 48<<10 {
		return DelegationInvocation{}, access.ErrLimit
	}
	sum := sha256.Sum256(encoded)
	binding := DelegationBinding{Version: 1, ID: grant.ID, ConversationID: grant.ConversationID, TurnID: grant.TurnID,
		Round: round, OperationKey: grant.TurnID + ":" + strconv.Itoa(round), PayloadSHA256: hex.EncodeToString(sum[:])}
	return DelegationInvocation{DelegationBinding: binding, RequestJSON: string(encoded)}, nil
}

func validDelegationBinding(binding DelegationBinding) bool {
	return binding.Version == 1 && delegationID(binding.ID) && delegationLabel(binding.ConversationID, 128) &&
		delegationLabel(binding.TurnID, 124) && binding.Round >= 0 && binding.Round < 4 &&
		binding.OperationKey == binding.TurnID+":"+strconv.Itoa(binding.Round) && len(binding.OperationKey) <= 128 && lowerDigest(binding.PayloadSHA256)
}

func (c *DelegationClient) Invoke(ctx context.Context, credential string, request DelegationInvocation) (DelegationOutcome, error) {
	if !delegationLabel(credential, 256) || !validDelegationBinding(request.DelegationBinding) || len(request.RequestJSON) == 0 || len(request.RequestJSON) > 48<<10 || !utf8.ValidString(request.RequestJSON) {
		return DelegationOutcome{}, access.ErrInvalid
	}
	sum := sha256.Sum256([]byte(request.RequestJSON))
	if hex.EncodeToString(sum[:]) != request.PayloadSHA256 {
		return DelegationOutcome{}, access.ErrConflict
	}
	response, err := c.bridge.Invoke(ctx, credential, request)
	return checkedDelegationOutcome(request.DelegationBinding, response, err)
}

// Status permits owner-authenticated recovery after expiry/revocation and never
// dispatches a model call. This client does not poll, retry Invoke or advance a
// round on a nonterminal outcome. Persist checkpoint monotonicity at the caller.
func (c *DelegationClient) Status(ctx context.Context, credential string, binding DelegationBinding) (DelegationOutcome, error) {
	if !delegationLabel(credential, 256) || !validDelegationBinding(binding) {
		return DelegationOutcome{}, access.ErrInvalid
	}
	response, err := c.bridge.Status(ctx, credential, binding)
	return checkedDelegationOutcome(binding, response, err)
}

func checkedDelegationOutcome(expected DelegationBinding, response DelegationOutcome, err error) (DelegationOutcome, error) {
	if err != nil {
		return DelegationOutcome{}, delegationBridgeError(err)
	}
	if response.DelegationBinding != expected || response.CheckpointSequence < 0 {
		return DelegationOutcome{}, access.ErrUnavailable
	}
	switch response.Status {
	case "reserved", "dispatched", "streaming", "unknown", "released":
		if response.Reply != nil || response.ChargedCredits != nil {
			return DelegationOutcome{}, access.ErrUnavailable
		}
	case "settled":
		if response.Reply == nil || response.ChargedCredits == nil || !validChargedCredits(*response.ChargedCredits) ||
			!utf8.ValidString(response.Reply.Text) || len(response.Reply.Text) > 8192 || response.Reply.Calls == nil || len(response.Reply.Calls) > 4 {
			return DelegationOutcome{}, access.ErrUnavailable
		}
		seen := map[string]bool{}
		for _, call := range response.Reply.Calls {
			var arguments map[string]any
			if !access.ID(call.ID) || len(call.ID) > 64 || !delegationLabel(call.Name, 128) || seen[call.ID] || len(call.Arguments) > 8192 || access.Decode(call.Arguments, &arguments) != nil || arguments == nil {
				return DelegationOutcome{}, access.ErrUnavailable
			}
			seen[call.ID] = true
		}
	default:
		return DelegationOutcome{}, access.ErrUnavailable
	}
	return response, nil
}

// Only a closed set of definite Core rejections crosses this boundary. Unknown
// transport failures (including cancellation/EOF after a possible commit) retain
// the original operation. Wrapped dependency text never reaches the caller.
func delegationBridgeError(err error) error {
	for _, definite := range []error{access.ErrInvalid, access.ErrUnauthorized, access.ErrForbidden, access.ErrConflict, access.ErrLimit, ErrDelegationBudget, ErrDelegationUnavailable} {
		if errors.Is(err, definite) {
			return definite
		}
	}
	return ErrDelegationUnknown
}

func validChargedCredits(value string) bool {
	if value == "" || len(value) > 16 || len(value) > 1 && value[0] == '0' {
		return false
	}
	for _, c := range value {
		if c < '0' || c > '9' {
			return false
		}
	}
	n, err := strconv.ParseInt(value, 10, 64)
	return err == nil && n <= 9007199254740991
}
