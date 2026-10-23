package composition

import (
	"context"
	"errors"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/coreclient"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/support"
)

func validateSupportCustomersSelection(cfg Config) error {
	configured := cfg.SupportCustomerSocket != "" || cfg.SupportCustomerTokenFile != "" || cfg.SupportNoteKeyFile != ""
	if !configured {
		return nil
	}
	selected := false
	for _, name := range cfg.Selected {
		selected = selected || name == "support"
	}
	if !selected || cfg.SupportCustomerSocket == "" || cfg.SupportCustomerTokenFile == "" || cfg.SupportNoteKeyFile == "" || cfg.SupportCustomerSocket == cfg.Socket {
		return errors.New("support customers require explicit support selection, a private HTTP socket and independent secrets")
	}
	return nil
}
func loadSupportCustomerCredentials(cfg Config, others ...[]byte) ([]byte, []byte, error) {
	if cfg.SupportCustomerTokenFile == "" {
		return nil, nil, nil
	}
	if len(cfg.HostCredential) < 32 {
		return nil, nil, errors.New("support customer projection requires independent host credential")
	}
	token, err := coreclient.ReadTokenFile(cfg.SupportCustomerTokenFile)
	if err != nil {
		return nil, nil, errors.New("private support projection credential unavailable")
	}
	key, err := coreclient.ReadPrivateFile(cfg.SupportNoteKeyFile, 32)
	if err != nil || len(key) != 32 {
		return nil, nil, errors.New("private support note key must contain exactly 32 bytes")
	}
	credentials := append([][]byte{cfg.HostCredential, token, key}, others...)
	if err = independentRuntimeCredentials(credentials...); err != nil {
		return nil, nil, err
	}
	return token, key, nil
}
func (r *Runtime) composeSupportCustomers(ctx context.Context, cfg Config, token, key []byte) error {
	if len(token) == 0 {
		return nil
	}
	if r.Support == nil {
		return errors.New("support service unavailable")
	}
	projection, err := support.NewCoreCustomerProjection(cfg.SupportCustomerSocket, token)
	if err != nil {
		return err
	}
	r.close = append(r.close, func() error { projection.Close(); return nil })
	if err = projection.Ready(ctx); err != nil {
		return errors.New("Core support customer projection capability unavailable")
	}
	r.Support, err = r.Support.WithCustomers(projection, key)
	return err
}
