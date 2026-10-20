package assistant

import (
	"context"
	"errors"
	"strings"
	"testing"
)

type giftPortFixture struct {
	receipt    *GiftReceipt
	get, claim int
	err        error
}

func (p *giftPortFixture) GetNewUserGift(context.Context, string) (*GiftReceipt, error) {
	p.get++
	return p.receipt, p.err
}
func (p *giftPortFixture) ClaimStoredNewUserGift(context.Context, string) (GiftReceipt, bool, error) {
	p.claim++
	if p.err != nil {
		return GiftReceipt{}, false, p.err
	}
	if p.receipt == nil {
		return GiftReceipt{}, false, ErrGiftUnknown
	}
	return *p.receipt, true, nil
}
func claimedGiftFixture() GiftReceipt {
	return GiftReceipt{ID: strings.Repeat("a", 64), OwnerUserID: 1, State: "claimed", CreditAmount: 10, CreatedAt: 100, ClaimedAt: 101, LedgerTransactionID: "7", MaxCreditAmount: 100, ClaimBlockedCode: "already_claimed"}
}

func TestGiftsReadNeverClaimsOrInfersEligibility(t *testing.T) {
	p := &giftPortFixture{}
	auth := &turnAuthorityFixture{}
	s, err := NewGifts(auth, p)
	if err != nil {
		t.Fatal(err)
	}
	value, err := s.GetNewUserGift(context.Background(), "session")
	if err != nil || value != nil || p.get != 1 || p.claim != 0 {
		t.Fatal(value, err, p)
	}
	r := claimedGiftFixture()
	p.receipt = &r
	value, err = s.GetNewUserGift(context.Background(), "session")
	if err != nil || value.Status != "claimed" || value.CreditAmount != 10 || value.Quota != 10 || value.AmountCents != 0 || value.AmountUSD != nil || p.claim != 0 {
		t.Fatal(value, err)
	}
	auth.revoked = true
	if _, err = s.GetNewUserGift(context.Background(), "session"); err == nil || p.get != 2 {
		t.Fatal("revoked Session reached Core port", err, p.get)
	}
}

func TestGiftsRejectWrongOwnerMalformedMoneyAndUnconfirmedClaim(t *testing.T) {
	good := claimedGiftFixture()
	for _, change := range []func(*GiftReceipt){func(r *GiftReceipt) { r.OwnerUserID = 2 }, func(r *GiftReceipt) { r.CreditAmount = 0 }, func(r *GiftReceipt) { r.LedgerTransactionID = "" }, func(r *GiftReceipt) { r.ClaimedAt = 0 }, func(r *GiftReceipt) { r.LedgerTransactionID = "07" }, func(r *GiftReceipt) { r.ClaimAvailable = true }, func(r *GiftReceipt) { r.ClaimBlockedCode = "private network text" }} {
		r := good
		change(&r)
		p := &giftPortFixture{receipt: &r}
		s, _ := NewGifts(&turnAuthorityFixture{}, p)
		if _, err := s.ClaimStoredNewUserGift(context.Background(), "session"); !errors.Is(err, ErrGiftUnknown) || p.claim != 1 {
			t.Fatal("invalid claimed receipt accepted", r, err)
		}
	}
	p := &giftPortFixture{err: ErrGiftUnknown}
	s, _ := NewGifts(&turnAuthorityFixture{}, p)
	if _, err := s.ClaimStoredNewUserGift(context.Background(), "session"); !errors.Is(err, ErrGiftUnknown) || p.claim != 1 || p.get != 0 {
		t.Fatal("uncertain claim retried", err, p)
	}
}

func TestGiftOfferCapIsExclusiveAndBlockedPolicyCannotPromiseClaim(t *testing.T) {
	r := GiftReceipt{ID: strings.Repeat("b", 64), OwnerUserID: 1, State: "offered", CreditAmount: 10, CreatedAt: 100, MaxCreditAmount: 10, ClaimAvailable: true}
	if _, err := checkedGift(1, r); err == nil {
		t.Fatal("exclusive cap accepted")
	}
	r.MaxCreditAmount = 11
	if _, err := checkedGift(1, r); err != nil {
		t.Fatal(err)
	}
	r.ClaimAvailable = false
	r.ClaimBlockedCode = "qualification_missing"
	if value, err := checkedGift(1, r); err != nil || value.ClaimAvailable {
		t.Fatal(value, err)
	}
	p := &giftPortFixture{receipt: &r}
	s, _ := NewGifts(&turnAuthorityFixture{}, p)
	if _, err := s.ClaimStoredNewUserGift(context.Background(), "session"); !errors.Is(err, ErrGiftUnknown) {
		t.Fatal("offer presented as claimed", err)
	}
}
