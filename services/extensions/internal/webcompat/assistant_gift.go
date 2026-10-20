package webcompat

import (
	"context"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"strings"

	"github.com/TokenNotIncluded/api.lmm.best/extensions/internal/modules/assistant"
	"github.com/TokenNotIncluded/api.lmm.best/extensions/internal/modules/assistant/access"
)

type AssistantGifts interface {
	GiftsAvailable() bool
	GetNewUserGift(context.Context, string) (*assistant.NewUserGift, error)
	ClaimStoredNewUserGift(context.Context, string) (assistant.GiftClaim, error)
}

func RegisterAssistantGifts(mux *http.ServeMux, auth *Authenticator, gifts AssistantGifts) {
	register := func(claim bool) http.HandlerFunc {
		return func(w http.ResponseWriter, r *http.Request) {
			if _, err := auth.Verify(r.Context(), r); err != nil {
				assistantRespond(w, nil, assistantAuthError(err))
				return
			}
			if access.Missing(gifts) || !gifts.GiftsAvailable() {
				assistantGiftFailure(w, "ASSISTANT_GIFT_UNAVAILABLE", "New-user gifts are not available on this installation.")
				return
			}
			query, err := assistantQuery(r)
			if err != nil || len(query) != 0 {
				assistantRespond(w, nil, access.ErrInvalid)
				return
			}
			// Both routes have an empty request. Even an existing offer cannot let
			// the browser choose an amount, recipient, payer or decision reference.
			body, err := io.ReadAll(http.MaxBytesReader(w, r.Body, 1024))
			if err != nil {
				assistantRespond(w, nil, access.ErrInvalid)
				return
			}
			if len(strings.TrimSpace(string(body))) != 0 {
				var empty struct{}
				if access.Decode(body, &empty) != nil {
					assistantRespond(w, nil, access.ErrInvalid)
					return
				}
			}
			token, _ := strings.CutPrefix(r.Header.Get("Authorization"), "Bearer ")
			var result any
			if claim {
				result, err = gifts.ClaimStoredNewUserGift(r.Context(), token)
			} else {
				result, err = gifts.GetNewUserGift(r.Context(), token)
			}
			if errors.Is(err, assistant.ErrGiftUnknown) {
				assistantGiftFailure(w, "ASSISTANT_GIFT_UNCONFIRMED", "Check the stored gift status before trying again.")
				return
			}
			if errors.Is(err, assistant.ErrDelegationUnavailable) {
				assistantGiftFailure(w, "ASSISTANT_GIFT_UNAVAILABLE", "New-user gifts are not available on this installation.")
				return
			}
			assistantRespond(w, result, err)
		}
	}
	mux.HandleFunc("GET /api/assistant/new-user-gift", register(false))
	mux.HandleFunc("POST /api/assistant/new-user-gift/claim", register(true))
}

func assistantGiftFailure(w http.ResponseWriter, code, message string) {
	w.Header().Set("Content-Type", "application/json; charset=utf-8")
	w.Header().Set("Cache-Control", "no-store")
	w.Header().Set("X-Content-Type-Options", "nosniff")
	w.WriteHeader(http.StatusServiceUnavailable)
	_ = json.NewEncoder(w).Encode(map[string]any{"success": false, "code": code, "retryable": false, "message": message})
}
