package webcompat

import (
	"context"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	pb "github.com/TokenNotIncluded/api.lmm.best/extensions/internal/corepb"
	"github.com/TokenNotIncluded/api.lmm.best/extensions/internal/modules/bounties"
	"github.com/TokenNotIncluded/api.lmm.best/extensions/internal/modules/wallet"
)

type builtinSessionCore struct {
	kind pb.CredentialKind
	seen string
}

func (c *builtinSessionCore) Authorize(_ context.Context, raw string) (*pb.AuthorizeResponse, error) {
	c.seen = raw
	return &pb.AuthorizeResponse{UserId: 7, PlatformLevel: 1, CredentialKind: c.kind, Owner: &pb.Account{Kind: pb.AccountKind_ACCOUNT_KIND_PERSONAL, Id: 7}}, nil
}

type builtinWalletAuthority struct {
	wallet.Authority
	seen  string
	calls int
}

func (a *builtinWalletAuthority) Balance(_ context.Context, raw string) (wallet.Balance, error) {
	a.calls++
	a.seen = raw
	return wallet.Balance{OwnerUserID: 7, AccountID: 91, CreditsPerUSD: 500000, AvailableCredits: 33}, nil
}

type builtinWalletRecords struct{ wallet.Records }
type builtinBountyRecords struct{ bounties.Repository }
type builtinBountyFunds struct{ bounties.Funds }

func TestBuiltinHTTPPreservesVerifiedSessionAndRejectsOtherCredentials(t *testing.T) {
	core := &builtinSessionCore{kind: pb.CredentialKind_CREDENTIAL_KIND_SESSION}
	host := []byte(strings.Repeat("h", 32))
	auth, e := NewAuthenticator(core, host)
	if e != nil {
		t.Fatal(e)
	}
	authority := &builtinWalletAuthority{}
	service, e := wallet.New(authority, builtinWalletRecords{}, "https://core.example.test")
	if e != nil {
		t.Fatal(e)
	}
	handlers, e := NewBuiltinHTTP(auth, service, &bounties.Service{Store: builtinBountyRecords{}, Funds: builtinBountyFunds{}}, "https://console.example.test", false)
	if e != nil {
		t.Fatal(e)
	}
	mux := http.NewServeMux()
	RegisterBuiltinHTTP(mux, handlers)
	session := "lmms_" + strings.Repeat("s", 40)
	request := httptest.NewRequest("GET", "https://console.example.test/api/wallet/balance", nil)
	request.Header.Set("Authorization", "Bearer "+session)
	request.Header.Set("X-User-ID", "999")
	response := httptest.NewRecorder()
	mux.ServeHTTP(response, request)
	if response.Code != 200 || core.seen != session || authority.seen != session || authority.calls != 1 {
		t.Fatalf("session was replaced: %d %s", response.Code, response.Body.String())
	}
	for _, kind := range []string{"host", "duplicate", "api-key"} {
		request = httptest.NewRequest("GET", "https://console.example.test/api/wallet/balance", nil)
		request.Header.Set("Authorization", "Bearer "+session)
		core.kind = pb.CredentialKind_CREDENTIAL_KIND_SESSION
		switch kind {
		case "host":
			request.Header.Set("Authorization", "Bearer "+string(host))
		case "duplicate":
			request.Header.Add("Authorization", "Bearer "+session)
		case "api-key":
			core.kind = pb.CredentialKind_CREDENTIAL_KIND_API_KEY
		}
		response = httptest.NewRecorder()
		mux.ServeHTTP(response, request)
		if response.Code < 400 || authority.calls != 1 {
			t.Fatalf("%s credential reached domain: %d", kind, response.Code)
		}
	}
}
func TestBuiltinHTTPPathsParticipateInHostAdmission(t *testing.T) {
	for _, path := range []string{"/api/wallet-transfer", "/api/wallet-transfer/42/cancel", "/api/wallet/balance", "/api/wallet/topup-link", "/api/open-source-bounties", "/api/open-source-bounties/projects/id"} {
		if !builtinHTTPPath(path) {
			t.Fatal(path)
		}
	}
	for _, path := range []string{"/api/wallet-transfer-other", "/api/wallet/balance-other", "/api/open-source-bounties-other"} {
		if builtinHTTPPath(path) {
			t.Fatal(path)
		}
	}
}
