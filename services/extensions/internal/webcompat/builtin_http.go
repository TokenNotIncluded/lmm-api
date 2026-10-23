package webcompat

import (
	"context"
	"errors"
	"net/http"
	"strings"

	"github.com/TokenNotIncluded/api.lmm.best/extensions/internal/modules/bounties"
	"github.com/TokenNotIncluded/api.lmm.best/extensions/internal/modules/store"
	"github.com/TokenNotIncluded/api.lmm.best/extensions/internal/modules/wallet"
)

type BuiltinHTTP struct {
	Wallet   http.Handler
	Bounties http.Handler
}

// NewBuiltinHTTP preserves the exact personal Session already verified by Core.
// Host credentials, market tokens, cookies and body identities are not upgraded
// to interactive authority. Each downstream domain authenticates it again.
func NewBuiltinHTTP(auth *Authenticator, walletService *wallet.Service, bountyService *bounties.Service, webOrigin string, localHTTP bool) (BuiltinHTTP, error) {
	var out BuiltinHTTP
	if auth == nil || walletService == nil || bountyService == nil {
		return out, store.ErrUnavailable
	}
	session := func(ctx context.Context, r *http.Request) (string, error) {
		if _, e := auth.Verify(ctx, r); e != nil {
			return "", e
		}
		raw, ok := strings.CutPrefix(r.Header.Get("Authorization"), "Bearer ")
		if !ok || raw == "" {
			return "", store.ErrUnauthorized
		}
		return raw, nil
	}
	walletHandler, e := wallet.NewHandlerWithLocalHTTP(walletService, func(ctx context.Context, r *http.Request) (string, error) {
		raw, e := session(ctx, r)
		if e == nil {
			return raw, nil
		}
		switch {
		case errors.Is(e, store.ErrUnauthorized):
			return "", wallet.ErrUnauthorized
		case errors.Is(e, store.ErrForbidden):
			return "", wallet.ErrForbidden
		default:
			return "", wallet.ErrUnknown
		}
	}, webOrigin, localHTTP)
	if e != nil {
		return out, e
	}
	bountyHandler, e := bounties.NewHTTP(bountyService, bounties.SessionExtractorFunc(func(r *http.Request) (string, error) {
		raw, e := session(r.Context(), r)
		if e == nil {
			return raw, nil
		}
		if errors.Is(e, store.ErrUnauthorized) || errors.Is(e, store.ErrForbidden) {
			return "", bounties.ErrForbidden
		}
		return "", bounties.ErrUnavailable
	}))
	if e != nil {
		return out, e
	}
	return BuiltinHTTP{walletHandler, bountyHandler}, nil
}

func RegisterBuiltinHTTP(mux *http.ServeMux, handlers BuiltinHTTP) {
	if !store.Missing(handlers.Wallet) {
		mux.Handle("/api/wallet-transfer", handlers.Wallet)
		mux.Handle("/api/wallet-transfer/", handlers.Wallet)
		mux.Handle("/api/wallet/balance", handlers.Wallet)
		mux.Handle("/api/wallet/topup-link", handlers.Wallet)
	}
	if !store.Missing(handlers.Bounties) {
		mux.Handle("/api/open-source-bounties", handlers.Bounties)
		mux.Handle("/api/open-source-bounties/", handlers.Bounties)
	}
}
func builtinHTTPPath(path string) bool {
	return path == "/api/wallet-transfer" || strings.HasPrefix(path, "/api/wallet-transfer/") || path == "/api/wallet/balance" || path == "/api/wallet/topup-link" || path == "/api/open-source-bounties" || strings.HasPrefix(path, "/api/open-source-bounties/")
}
