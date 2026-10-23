package coreclient

import (
	"context"

	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
)

func (c *AssistantClient) assistantGiftReady(ctx context.Context, user string) error {
	if !validUserCredential(user) {
		return status.Error(codes.Unauthenticated, "invalid user credential")
	}
	return c.GiftReady(ctx)
}

// No mutation DTO is accepted: Core selects only this Session's stored offer.
func (c *AssistantClient) AssistantGetNewUserGift(ctx context.Context, user string) (*pb.AssistantGiftStatus, error) {
	if err := c.assistantGiftReady(ctx, user); err != nil {
		return nil, err
	}
	r, err := pb.NewCoreAssistantModelClient(c.rpc.connection).GetNewUserGift(c.rpc.context(ctx, user), &pb.AssistantGiftRequest{})
	return r, assistantReply(r, err)
}

func (c *AssistantClient) AssistantClaimStoredNewUserGift(ctx context.Context, user string) (*pb.AssistantGiftClaim, error) {
	if err := c.assistantGiftReady(ctx, user); err != nil {
		return nil, err
	}
	r, err := pb.NewCoreAssistantModelClient(c.rpc.connection).ClaimStoredNewUserGift(c.rpc.context(ctx, user), &pb.AssistantGiftRequest{})
	return r, assistantReply(r, err)
}
