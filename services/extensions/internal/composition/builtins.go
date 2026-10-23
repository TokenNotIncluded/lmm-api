package composition

import (
	"bytes"
	"context"
	"database/sql"
	"errors"
	"path/filepath"
	"strings"

	"github.com/TokenNotIncluded/lmm-api/services/extensions/internal/modules/bounties"
	"github.com/TokenNotIncluded/lmm-api/services/extensions/internal/modules/toolmarket"
	"github.com/TokenNotIncluded/lmm-api/services/extensions/internal/modules/toolmarket/rustbridge"
	"github.com/TokenNotIncluded/lmm-api/services/extensions/internal/modules/wallet"
)

// BuiltinDependencies contains already-loaded purpose credentials. The caller
// validates them against all other host credentials before constructing modules.
// The database is an extension records database; this function executes no DDL.
type BuiltinDependencies struct {
	Database         *sql.DB
	ExpectedDatabase string
	Gate             toolmarket.BuiltinGate
	WalletSocket     string
	WalletToken      []byte
	BountySocket     string
	BountyToken      []byte
	ImageSocket      string
	ImageToken       []byte
	CorePublicOrigin string
	LocalHTTP        bool
}

func composeBuiltinDispatcher(ctx context.Context, in BuiltinDependencies) (*toolmarket.BuiltinDispatcher, func(), error) {
	if in.Database == nil || in.ExpectedDatabase == "" || in.Gate == nil {
		return nil, nil, errors.New("builtin records and authority unavailable")
	}
	tokens := [][]byte{in.WalletToken, in.BountyToken, in.ImageToken}
	sockets := []string{in.WalletSocket, in.BountySocket, in.ImageSocket}
	for i := range tokens {
		validToken := len(tokens[i]) >= 32 && len(tokens[i]) <= 256
		for _, b := range tokens[i] {
			validToken = validToken && b >= 33 && b <= 126
		}
		if !validToken || !filepath.IsAbs(sockets[i]) || filepath.Clean(sockets[i]) != sockets[i] || strings.ContainsAny(sockets[i], "\x00\r\n") || sockets[i] != sockets[0] {
			return nil, nil, errors.New("builtin purpose configuration unavailable")
		}
		for j := 0; j < i; j++ {
			if bytes.Equal(tokens[i], tokens[j]) {
				return nil, nil, errors.New("builtin authority credentials must be independent")
			}
		}
	}
	// Missing grants/schema are startup errors, never empty-record success.
	var walletReady bool
	if e := in.Database.QueryRowContext(ctx, `SELECT has_schema_privilege(current_user,'extension_wallet','USAGE') AND has_table_privilege(current_user,'extension_wallet.transfer_links','SELECT') AND has_table_privilege(current_user,'extension_wallet.transfer_links','INSERT')`).Scan(&walletReady); e != nil || !walletReady {
		return nil, nil, errors.New("wallet records installation unavailable")
	}
	bountyRecords, e := bounties.NewPostgres(ctx, in.Database, in.ExpectedDatabase)
	if e != nil {
		return nil, nil, e
	}
	walletRecords, e := wallet.NewPostgresRecords(in.Database)
	if e != nil {
		return nil, nil, e
	}
	walletCore, e := wallet.NewCoreClient(in.WalletSocket, in.WalletToken)
	if e != nil {
		return nil, nil, e
	}
	closeWallet := func() { walletCore.Close() }
	walletService, e := wallet.NewWithLocalHTTP(walletCore, walletRecords, in.CorePublicOrigin, in.LocalHTTP)
	if e != nil {
		closeWallet()
		return nil, nil, e
	}
	bountyCore, e := bounties.NewCoreClient(in.BountySocket, string(in.BountyToken))
	if e != nil {
		closeWallet()
		return nil, nil, e
	}
	drawing, e := rustbridge.NewDrawing(in.ImageSocket, in.ImageToken, in.CorePublicOrigin, in.LocalHTTP)
	if e != nil {
		closeWallet()
		return nil, nil, e
	}
	close := func() { drawing.Close(); closeWallet() }
	dispatcher := &toolmarket.BuiltinDispatcher{Gate: in.Gate, Wallet: walletService, Bounties: &bounties.Service{Store: bountyRecords, Funds: bountyCore}, Drawing: drawing, CorePublicOrigin: in.CorePublicOrigin}
	if e := dispatcher.CheckReadiness(ctx); e != nil {
		close()
		return nil, nil, errors.New("live Core builtin authorities or compiled registry unavailable")
	}
	return dispatcher, close, nil
}
