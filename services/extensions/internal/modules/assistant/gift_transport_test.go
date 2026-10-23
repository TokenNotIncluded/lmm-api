package assistant

import (
	"context"
	"errors"
	"strings"
	"testing"

	pb "github.com/TokenNotIncluded/api.lmm.best/extensions/internal/corepb"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
)

// Controlled peer tests cover actual UDS/protobuf/host+Session metadata and
// closed decoding. They are not Core PostgreSQL eligibility or ledger evidence.
type giftRPCCounts struct{ reads, claims int }

func (s *assistantRPCFixture) giftCalls() giftRPCCounts {
	s.mu.Lock()
	defer s.mu.Unlock()
	return giftRPCCounts{s.giftReads, s.giftClaims}
}

func giftRPCReceiptFixture() *pb.AssistantGiftReceipt {
	return &pb.AssistantGiftReceipt{GiftId: strings.Repeat("a", 64), OwnerUserId: 1, State: pb.AssistantGiftState_ASSISTANT_GIFT_STATE_CLAIMED, CreditAmount: 10, CreatedAtUnix: 100, ClaimedAtUnix: 101, LedgerTransactionId: "7", MaxCreditAmount: 100, ClaimBlockedCode: "already_claimed"}
}

func (s *assistantRPCFixture) GetNewUserGift(ctx context.Context, _ *pb.AssistantGiftRequest) (*pb.AssistantGiftStatus, error) {
	if err := s.check(ctx, true); err != nil {
		return nil, err
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	s.giftReads++
	if s.giftMode == "none" {
		return &pb.AssistantGiftStatus{}, nil
	}
	r := &pb.AssistantGiftStatus{Gift: giftRPCReceiptFixture()}
	if s.giftMode == "unknown-field" {
		r.ProtoReflect().SetUnknown([]byte{0x98, 0x06, 0x01})
	}
	return r, nil
}

func (s *assistantRPCFixture) ClaimStoredNewUserGift(ctx context.Context, _ *pb.AssistantGiftRequest) (*pb.AssistantGiftClaim, error) {
	if err := s.check(ctx, true); err != nil {
		return nil, err
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	s.giftClaims++
	if s.giftMode == "lost-ack" {
		return nil, status.Error(codes.Unavailable, "private storage or credential detail")
	}
	r := &pb.AssistantGiftClaim{Gift: giftRPCReceiptFixture(), AlreadyClaimed: true}
	if s.giftMode == "unknown-enum" {
		r.Gift.State = pb.AssistantGiftState(99)
	}
	if s.giftMode == "no-ledger" {
		r.Gift.LedgerTransactionId = ""
	}
	return r, nil
}

func TestGiftUDSNeedsIndependentCapabilityAndCarriesOriginalSession(t *testing.T) {
	peer := &assistantRPCFixture{feature: true, major: 1}
	_, client := newAssistantRPCFixture(t, peer)
	port, _ := NewPersonalGiftPort(client)
	if _, err := port.GetNewUserGift(context.Background(), assistantSessionFixture); !errors.Is(err, ErrDelegationUnavailable) || peer.giftCalls().reads != 0 {
		t.Fatal("text implied gift read", err)
	}
	if _, _, err := port.ClaimStoredNewUserGift(context.Background(), assistantSessionFixture); !errors.Is(err, ErrDelegationUnavailable) || peer.giftCalls().claims != 0 {
		t.Fatal("text implied gift claim", err)
	}
	peer = &assistantRPCFixture{giftFeature: true, major: 1}
	_, client = newAssistantRPCFixture(t, peer)
	port, _ = NewPersonalGiftPort(client)
	value, err := port.GetNewUserGift(context.Background(), assistantSessionFixture)
	if err != nil || value == nil || value.CreditAmount != 10 || value.OwnerUserID != 1 || peer.giftCalls().reads != 1 {
		t.Fatal(value, err)
	}
	valueClaim, already, err := port.ClaimStoredNewUserGift(context.Background(), assistantSessionFixture)
	if err != nil || !already || valueClaim.State != "claimed" || peer.giftCalls().claims != 1 {
		t.Fatal(valueClaim, already, err)
	}
}

func TestGiftUDSMalformedOrUnknownACKNeverRetries(t *testing.T) {
	for _, mode := range []string{"lost-ack", "unknown-enum", "no-ledger"} {
		t.Run(mode, func(t *testing.T) {
			peer := &assistantRPCFixture{giftFeature: true, major: 1, giftMode: mode}
			_, client := newAssistantRPCFixture(t, peer)
			port, _ := NewPersonalGiftPort(client)
			if _, _, err := port.ClaimStoredNewUserGift(context.Background(), assistantSessionFixture); !errors.Is(err, ErrGiftUnknown) || peer.giftCalls().claims != 1 || peer.giftCalls().reads != 0 {
				t.Fatal("bad ACK repeated a claim", err, peer.giftCalls().claims, peer.giftCalls().reads)
			}
		})
	}
	peer := &assistantRPCFixture{giftFeature: true, major: 1, giftMode: "unknown-field"}
	_, client := newAssistantRPCFixture(t, peer)
	port, _ := NewPersonalGiftPort(client)
	if _, err := port.GetNewUserGift(context.Background(), assistantSessionFixture); !errors.Is(err, ErrGiftUnknown) {
		t.Fatal(err)
	}
	peer = &assistantRPCFixture{giftFeature: true, major: 1, giftMode: "none"}
	_, client = newAssistantRPCFixture(t, peer)
	port, _ = NewPersonalGiftPort(client)
	if value, err := port.GetNewUserGift(context.Background(), assistantSessionFixture); err != nil || value != nil || peer.giftCalls().claims != 0 {
		t.Fatal(value, err)
	}
}
