package coreclient

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/json"
	"io"
	"reflect"
	"unicode/utf8"

	pb "github.com/TokenNotIncluded/api.lmm.best/extensions/internal/corepb"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
)

const AssistantToolConfirmationFeature = "assistant.tool_confirmation.v1"
const maxConfirmationDraft = 16 << 10

func (c *AssistantClient) ConfirmationReady(ctx context.Context) error {
	return c.assistantFeatures(ctx, AssistantToolConfirmationFeature)
}
func (c *AssistantClient) confirmationReady(ctx context.Context, user string) error {
	if !validUserCredential(user) {
		return status.Error(codes.Unauthenticated, "invalid user credential")
	}
	return c.ConfirmationReady(ctx)
}

// Decode bounded object JSON while rejecting duplicate keys and excessive depth.
// Numbers retain their decimal spelling, avoiding float64 precision loss.
func confirmationObject(data []byte) (any, bool) {
	return boundedAssistantObject(data, maxConfirmationDraft)
}

func boundedAssistantObject(data []byte, maximum int) (any, bool) {
	if len(data) == 0 || len(data) > maximum || !utf8.Valid(data) {
		return nil, false
	}
	d := json.NewDecoder(bytes.NewReader(data))
	d.UseNumber()
	var value func(int) (any, bool)
	value = func(depth int) (any, bool) {
		if depth > 64 {
			return nil, false
		}
		tok, err := d.Token()
		if err != nil {
			return nil, false
		}
		delim, ok := tok.(json.Delim)
		if !ok {
			return tok, true
		}
		switch delim {
		case '{':
			out := map[string]any{}
			for d.More() {
				key, err := d.Token()
				if err != nil {
					return nil, false
				}
				name, ok := key.(string)
				if !ok {
					return nil, false
				}
				if _, exists := out[name]; exists {
					return nil, false
				}
				v, ok := value(depth + 1)
				if !ok {
					return nil, false
				}
				out[name] = v
			}
			end, err := d.Token()
			return out, err == nil && end == json.Delim('}')
		case '[':
			out := []any{}
			for d.More() {
				v, ok := value(depth + 1)
				if !ok {
					return nil, false
				}
				out = append(out, v)
			}
			end, err := d.Token()
			return out, err == nil && end == json.Delim(']')
		default:
			return nil, false
		}
	}
	out, ok := value(0)
	if !ok {
		return nil, false
	}
	if _, ok = out.(map[string]any); !ok {
		return nil, false
	}
	_, err := d.Token()
	return out, err == io.EOF
}
func confirmationResponse(r *pb.AssistantToolConfirmation, operation string) (any, bool) {
	if !assistantMessage(r) || !assistantDigest(r.GetChallengeId(), "") || r.GetToolOperationId() != operation || !agentIdentifier.MatchString(operation) || !assistantLabel(r.GetToolName(), 80) || !assistantLabel(r.GetToolVersion(), 128) || len(r.GetSourceArgumentsSha256()) != sha256.Size || len(r.GetNormalizedDraftSha256()) != sha256.Size || r.GetExpiresAtUnix() <= 0 || r.GetExecuted() || r.GetBrowserPath() != "/api/user/assistant/confirmations/"+r.GetChallengeId() {
		return nil, false
	}
	switch r.GetState() {
	case "pending", "approved", "rejected":
	default:
		return nil, false
	}
	for _, v := range []string{r.GetResourceKind(), r.GetResourceId(), r.GetResourceVersion()} {
		if v != "" && !assistantLabel(v, 128) {
			return nil, false
		}
	}
	preview, ok := confirmationObject(r.GetPreviewJson())
	if !ok {
		return nil, false
	}
	h := sha256.Sum256(r.GetPreviewJson())
	if !bytes.Equal(h[:], r.GetNormalizedDraftSha256()) {
		return nil, false
	}
	return preview, true
}

// These two methods cannot approve a challenge or execute a domain effect.
// Source argument binding is compared by the consumer against its immutable
// Core tool call; Go never supplies a source digest, actor or confirmation flag.
func (c *AssistantClient) PrepareAssistantToolConfirmation(ctx context.Context, user string, r *pb.PrepareAssistantToolConfirmationRequest) (*pb.AssistantToolConfirmation, error) {
	if !assistantMessage(r) || !agentIdentifier.MatchString(r.GetToolOperationId()) {
		return nil, agentInvalid()
	}
	draft, ok := confirmationObject(r.GetNormalizedDraftJson())
	if !ok {
		return nil, agentInvalid()
	}
	if err := c.confirmationReady(ctx, user); err != nil {
		return nil, err
	}
	out, err := pb.NewCoreAssistantModelClient(c.rpc.connection).PrepareAssistantToolConfirmation(c.rpc.context(ctx, user), r)
	if err != nil {
		return nil, err
	}
	preview, ok := confirmationResponse(out, r.GetToolOperationId())
	if !ok || !reflect.DeepEqual(draft, preview) {
		return nil, agentCorrupt()
	}
	return out, nil
}
func (c *AssistantClient) GetAssistantToolConfirmationReceipt(ctx context.Context, user string, r *pb.GetAssistantToolConfirmationReceiptRequest) (*pb.AssistantToolConfirmation, error) {
	if !assistantMessage(r) || !agentIdentifier.MatchString(r.GetToolOperationId()) || len(r.GetNormalizedDraftSha256()) != sha256.Size {
		return nil, agentInvalid()
	}
	if err := c.confirmationReady(ctx, user); err != nil {
		return nil, err
	}
	out, err := pb.NewCoreAssistantModelClient(c.rpc.connection).GetAssistantToolConfirmationReceipt(c.rpc.context(ctx, user), r)
	if err != nil {
		return nil, err
	}
	if _, ok := confirmationResponse(out, r.GetToolOperationId()); !ok || out.GetState() != "approved" || !bytes.Equal(out.GetNormalizedDraftSha256(), r.GetNormalizedDraftSha256()) {
		return nil, agentCorrupt()
	}
	return out, nil
}
