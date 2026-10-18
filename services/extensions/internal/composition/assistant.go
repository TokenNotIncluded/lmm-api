package composition

import (
	"context"
	"database/sql"
	"errors"

	"github.com/TokenNotIncluded/api.lmm.best/extensions/internal/coreclient"
	"github.com/TokenNotIncluded/api.lmm.best/extensions/internal/modules/assistant"
	"github.com/TokenNotIncluded/api.lmm.best/extensions/internal/modules/assistant/access"
)

func validateAssistantSelection(cfg Config) error {
	if cfg.AssistantTokenFile == "" {
		return nil
	}
	for _, name := range cfg.Selected {
		if name == "assistant" {
			return nil
		}
	}
	return errors.New("assistant RPC configuration requires explicit assistant selection")
}

// Read once, compare the exact loaded bytes, then pass those same bytes to the
// dedicated client. Missing optional config leaves every new port unavailable.
func loadAssistantCredential(cfg Config, identity, commerce, payment []byte) ([]byte, error) {
	if cfg.AssistantTokenFile == "" {
		return nil, nil
	}
	if len(cfg.HostCredential) < 32 {
		return nil, errors.New("assistant RPC requires an independent loaded host credential")
	}
	token, err := coreclient.ReadTokenFile(cfg.AssistantTokenFile)
	if err != nil {
		return nil, errors.New("private assistant RPC credential unavailable")
	}
	if err = independentRuntimeCredentials(cfg.HostCredential, identity, commerce, payment, token); err != nil {
		return nil, err
	}
	return token, nil
}

func (r *Runtime) composeAssistantDelegation(ctx context.Context, cfg Config, db *sql.DB, authority access.Authority, token []byte) error {
	if len(token) == 0 {
		return nil
	}
	client, err := coreclient.NewAssistantClient(cfg.Socket, token, r.Client)
	if err != nil {
		return err
	}
	// Until the real Core authority is mounted AND advertises the frozen
	// capability, leave both entry points nil. No startup DDL or fallback.
	if client.Ready(ctx) != nil {
		_ = client.Close()
		return nil
	}
	r.AssistantModelClient = client
	r.close = append(r.close, client.Close)
	journal, err := assistant.NewDelegationPostgresJournal(ctx, db, cfg.Assistant.Database)
	if err != nil {
		return err
	}
	port, err := assistant.NewPersonalTextPort(client)
	if err != nil {
		return err
	}
	r.AssistantPending, err = assistant.NewPendingConversations(authority, journal, port)
	if err != nil {
		return err
	}
	r.AssistantTurns, err = assistant.NewDelegatedTurns(authority, journal, port)
	// Assistant itself still receives nil Model: reservation is no reply port.
	return err
}
