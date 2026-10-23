package composition

import (
	"context"
	"errors"
	"net/http"

	"github.com/TokenNotIncluded/lmm-api/services/extensions/internal/coreclient"
	"github.com/TokenNotIncluded/lmm-api/services/extensions/internal/modules/toolmarket"
	"github.com/TokenNotIncluded/lmm-api/services/extensions/internal/modules/toolmarket/rustbridge"
)

func validateToolMarketSelection(cfg Config) error {
	if cfg.ToolMarketCorePublicOrigin != "" {
		if cfg.HostPublicOrigin == "" {
			return errors.New("market MCP requires a configured Go public origin")
		}
		if err := ValidateCorePublicOrigin(cfg.LocalHTTP, cfg.HostListen, cfg.HostPublicOrigin, cfg.ToolMarketCorePublicOrigin); err != nil {
			return err
		}
		if cfg.ToolMarketBuiltins.configured() && cfg.ToolMarketCorePublicOrigin != cfg.ToolMarketBuiltins.CorePublicOrigin {
			return errors.New("market Core origins must match")
		}
	}
	if err := validateToolMarketCallbackProfile(cfg); err != nil {
		return err
	}
	if err := validateToolMarketWitness(cfg); err != nil {
		return err
	}
	if err := cfg.ToolMarketBuiltins.validate(cfg); err != nil {
		return err
	}
	if cfg.ToolMarketMeteringFile != "" {
		return errors.New("toolmarket supplier verification keys must be configured in Core")
	}
	selected := false
	for _, name := range cfg.Selected {
		selected = selected || name == "toolmarket"
	}
	configured := cfg.ToolMarketCorePublicOrigin != "" || cfg.ToolMarketWitnessSocket != "" || cfg.ToolMarketWitnessTokenFile != "" || cfg.ToolMarketBuiltins.configured() || cfg.ToolMarketMeteringFile != "" || cfg.ToolMarket.URLFile != "" || cfg.ToolMarket.Database != "" || cfg.ToolMarketKeyFile != "" || cfg.ToolMarketCallbackURL != "" || cfg.ToolMarketSocket != "" || cfg.ToolMarketTokenFile != ""
	if !selected {
		if configured {
			return errors.New("toolmarket configuration requires explicit selection")
		}
		return nil
	}
	if !configured || cfg.ToolMarketKeyFile == "" || cfg.ToolMarketCallbackURL == "" || cfg.ToolMarketSocket == "" || cfg.ToolMarketTokenFile == "" || cfg.ToolMarketSocket == cfg.Socket {
		return errors.New("toolmarket dedicated runtime configuration unavailable")
	}
	return cfg.ToolMarket.validate()
}
func (r *Runtime) composeToolMarket(ctx context.Context, cfg Config, otherCredentials ...[]byte) error {
	token, err := coreclient.ReadTokenFile(cfg.ToolMarketTokenFile)
	if err != nil {
		return errors.New("private toolmarket Core credential unavailable")
	}
	key, err := coreclient.ReadPrivateFile(cfg.ToolMarketKeyFile, 32)
	if err != nil || len(key) != 32 {
		return errors.New("private toolmarket encryption key must contain exactly 32 bytes")
	}
	if len(cfg.HostCredential) < 32 {
		return errors.New("toolmarket requires loaded independent host credential")
	}
	witnessToken, err := loadToolMarketWitness(cfg)
	if err != nil {
		return err
	}
	builtinTokens, err := loadToolMarketBuiltinTokens(cfg.ToolMarketBuiltins)
	if err != nil {
		return err
	}
	credentials := append([][]byte{cfg.HostCredential, token, key}, otherCredentials...)
	credentials = append(credentials, builtinTokens...)
	if len(witnessToken) != 0 {
		credentials = append(credentials, witnessToken)
	}
	if err := independentRuntimeCredentials(credentials...); err != nil {
		return err
	}
	r.retainPaymentIsolationCredentials(cfg.Selected, credentials...)
	bundle := defaultStorageBundle
	if cfg.ToolMarketBuiltins.configured() {
		bundle = toolMarketBuiltinsBundle
	}
	db, err := openStorageBundle(ctx, cfg.ToolMarket, bundle)
	if err != nil {
		return err
	}
	r.close = append(r.close, db.Close)
	snapshot, err := toolmarket.OpenPostgresStore(ctx, db, key)
	if err != nil {
		return errors.New("existing encrypted toolmarket storage unavailable")
	}
	r.close = append(r.close, snapshot.Close)
	bridge, err := rustbridge.New(cfg.ToolMarketSocket, token)
	if err != nil {
		return err
	}
	r.close = append(r.close, func() error { bridge.Close(); return nil })
	if err := bridge.Ready(ctx); err != nil {
		return errors.New("toolmarket Core capabilities unavailable")
	}
	if err := bridge.LegacyReady(ctx, cfg.ToolMarketCorePublicOrigin != ""); err != nil {
		return errors.New("toolmarket credit authority capabilities unavailable")
	}
	if len(witnessToken) != 0 {
		if err := bridge.WithFixedWitness(cfg.ToolMarketWitnessSocket, witnessToken); err != nil {
			return err
		}
		if err := bridge.FixedWitnessReady(ctx); err != nil {
			return errors.New("reviewed fixed-price witness authority unavailable")
		}
	}
	var builtin toolmarket.LegacyBuiltinRuntime
	if len(builtinTokens) != 0 {
		dispatcher, closeBuiltin, err := composeBuiltinDispatcher(ctx, BuiltinDependencies{Database: db, ExpectedDatabase: cfg.ToolMarket.Database, Gate: bridge, WalletSocket: cfg.ToolMarketBuiltins.WalletSocket, WalletToken: builtinTokens[0], BountySocket: cfg.ToolMarketBuiltins.BountySocket, BountyToken: builtinTokens[1], ImageSocket: cfg.ToolMarketBuiltins.ImageSocket, ImageToken: builtinTokens[2], CorePublicOrigin: cfg.ToolMarketBuiltins.CorePublicOrigin, LocalHTTP: cfg.LocalHTTP})
		if err != nil {
			return err
		}
		r.close = append(r.close, func() error { closeBuiltin(); return nil })
		builtin = dispatcher
		r.Builtins = dispatcher
	}
	r.ToolMarket, err = toolmarket.New(snapshot, bridge, toolmarket.Config{LocalHTTP: cfg.LocalHTTP, BuiltinRuntime: builtin, CallbackURL: cfg.ToolMarketCallbackURL, Funds: bridge})
	if err == nil && cfg.ToolMarketCorePublicOrigin != "" {
		mux := http.NewServeMux()
		err = r.ToolMarket.MountLegacyMarketMCP(mux, toolmarket.LegacyMarketMCPOptions{LocalHTTP: cfg.LocalHTTP, Issuer: cfg.ToolMarketCorePublicOrigin + "/api/oauth2/market", Resource: cfg.HostPublicOrigin + "/mcp/market", ResourceMetadataURL: cfg.ToolMarketCorePublicOrigin + "/.well-known/oauth-protected-resource/mcp/market"})
		if err == nil {
			r.ToolMarketMCP = mux
		}
	}
	return err
}
