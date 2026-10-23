package webcompat

import (
	"context"
	"net/http"
	"strings"
	"testing"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant"
)

type assistantGiftFixture struct {
	assistantChatFixture
	reads, claims int
	uncertain     bool
}

func (p *assistantGiftFixture) GiftsAvailable() bool { return true }
func (p *assistantGiftFixture) GetNewUserGift(context.Context, string) (*assistant.NewUserGift, error) {
	p.reads++
	return nil, nil
}
func (p *assistantGiftFixture) ClaimStoredNewUserGift(context.Context, string) (assistant.GiftClaim, error) {
	p.claims++
	if p.uncertain {
		return assistant.GiftClaim{}, assistant.ErrGiftUnknown
	}
	return assistant.GiftClaim{Gift: assistant.NewUserGift{Status: "claimed", CreditAmount: 10, Quota: 10, CreatedAt: 100, ClaimedAt: 101}, AlreadyClaimed: true}, nil
}

func TestAssistantGiftRoutesConsumeOnlyTrustedStoredReceiptAndEmptyRequest(t *testing.T) {
	p := &assistantGiftFixture{}
	mux := http.NewServeMux()
	RegisterAssistant(mux, newAuth(t, &authCore{}), &assistantPort{}, p)
	w := assistantRequest(mux, "GET", "/api/assistant/new-user-gift", webToken, "")
	if w.Code != 200 || !strings.Contains(w.Body.String(), `"data":null`) || p.reads != 1 || p.claims != 0 {
		t.Fatal(w.Code, w.Body.String(), p)
	}
	w = assistantRequest(mux, "POST", "/api/assistant/new-user-gift/claim", webToken, "")
	if w.Code != 200 || !strings.Contains(w.Body.String(), `"already_claimed":true`) || p.claims != 1 {
		t.Fatal(w.Code, w.Body.String(), p.claims)
	}
	for _, body := range []string{`{"amount_credits":10}`, `{"owner_user_id":2}`, `{"payer":1}`, `{"decision_id":"arbitrary"}`} {
		w = assistantRequest(mux, "POST", "/api/assistant/new-user-gift/claim", webToken, body)
		if w.Code != 400 || p.claims != 1 {
			t.Fatal(w.Code, w.Body.String(), p.claims)
		}
	}
	w = assistantRequest(mux, "GET", "/api/assistant/new-user-gift?owner=2", webToken, "")
	if w.Code != 400 || p.reads != 1 {
		t.Fatal(w.Code, w.Body.String(), p.reads)
	}
}

func TestAssistantGiftUnknownClaimDoesNotRetryOrInventCredit(t *testing.T) {
	p := &assistantGiftFixture{uncertain: true}
	mux := http.NewServeMux()
	RegisterAssistant(mux, newAuth(t, &authCore{}), &assistantPort{}, p)
	w := assistantRequest(mux, "POST", "/api/assistant/new-user-gift/claim", webToken, "")
	if w.Code != 503 || !strings.Contains(w.Body.String(), "ASSISTANT_GIFT_UNCONFIRMED") || strings.Contains(w.Body.String(), `"gift"`) || p.claims != 1 || p.reads != 0 {
		t.Fatal(w.Code, w.Body.String(), p)
	}
}
