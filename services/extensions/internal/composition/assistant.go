package composition

import (
	"context"
	"database/sql"
	"errors"

	"github.com/TokenNotIncluded/lmm-api/services/extensions/internal/coreclient"
	"github.com/TokenNotIncluded/lmm-api/services/extensions/internal/modules/assistant"
	"github.com/TokenNotIncluded/lmm-api/services/extensions/internal/modules/assistant/access"
	"github.com/TokenNotIncluded/lmm-api/services/extensions/internal/modules/assistant/personalization"
	"github.com/TokenNotIncluded/lmm-api/services/extensions/internal/modules/assistant_market"
	"github.com/TokenNotIncluded/lmm-api/services/extensions/internal/modules/assistant_services"
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
	// Require both durable admission and authoritative answer execution before
	// exposing chat. An old admission-only Core leaves all new ports nil.
	textReady, agentReady := client.Ready(ctx) == nil, client.AgentReady(ctx) == nil
	if !textReady && !agentReady {
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
	var registration assistant.ConversationRegistrationPort = port
	if agentReady {
		registration, err = assistant.NewCoreAgentTurnPort(client)
		if err != nil {
			return err
		}
	}
	r.AssistantPending, err = assistant.NewPendingConversations(authority, journal, registration)
	if err != nil {
		return err
	}
	if textReady {
		r.AssistantTurns, err = assistant.NewDelegatedTurns(authority, journal, port)
		if err != nil {
			return err
		}
	}
	var gifts *assistant.Gifts
	if client.GiftReady(ctx) == nil {
		giftPort, giftErr := assistant.NewPersonalGiftPort(client)
		if giftErr != nil {
			return giftErr
		}
		gifts, giftErr = assistant.NewGifts(authority, giftPort)
		if giftErr != nil {
			return giftErr
		}
	}
	if agentReady {
		agentPort, portErr := assistant.NewCoreAgentTurnPort(client)
		if portErr != nil {
			return portErr
		}
		agentJournal, journalErr := assistant.NewAgentPostgresJournal(ctx, db, cfg.Assistant.Database)
		if journalErr != nil {
			return journalErr
		}
		dependencies := assistant_services.Dependencies{}
		if cfg.AssistantServicesDependencies != nil {
			var dependencyErr error
			dependencies, dependencyErr = cfg.AssistantServicesDependencies(ctx, client)
			if dependencyErr != nil {
				return dependencyErr
			}
		}
		if access.Missing(dependencies.Confirmations) {
			confirmation, confirmationErr := assistant_services.NewCoreConfirmationAuthority(client, authority)
			if confirmationErr != nil {
				return confirmationErr
			}
			if confirmation.Ready(ctx) == nil {
				dependencies.Confirmations = confirmation
			}
		}
		if access.Missing(dependencies.Policies) {
			dependencies.Policies = cfg.AssistantServicesPolicies
		}
		if factsClient, ok := any(client).(assistant_services.BusinessFactsClient); ok {
			facts, factsErr := assistant_services.NewCoreBusinessFacts(factsClient, authority)
			if factsErr != nil {
				return factsErr
			}
			if facts.Ready(ctx) == nil {
				if access.Missing(dependencies.SupportEligibility) {
					dependencies.SupportEligibility = facts
				}
				if access.Missing(dependencies.Public) && !access.Missing(cfg.AssistantPublicSnapshot) {
					guides, guidesErr := assistant_services.NewPublicGuides(authority, cfg.AssistantPublicSnapshot, facts)
					if guidesErr != nil {
						return guidesErr
					}
					dependencies.Public = guides
				}
			}
		}
		domain, domainErr := assistant_services.New(ctx, db, authority, dependencies)
		if domainErr != nil {
			return domainErr
		}
		personal, personalErr := personalization.New(domain, r.AssistantHistory)
		if personalErr != nil {
			return personalErr
		}
		bindings := append(personal.Domain().AvailableBindings(), personal.AvailableBindings()...)
		r.AssistantServices = personal.Domain()
		if r.ToolMarket != nil {
			market, marketErr := assistant_market.New(r.ToolMarket)
			if marketErr != nil {
				return marketErr
			}
			marketBindings, bindingErr := market.Bindings()
			if bindingErr != nil {
				return bindingErr
			}
			bindings = append(bindings, marketBindings...)
		}
		registry, registryErr := assistant.NewAgentRegistry(bindings)
		if registryErr != nil {
			return registryErr
		}
		available := make([]coreclient.AssistantExtensionBinding, 0, len(bindings))
		for _, descriptor := range registry.Descriptors() {
			available = append(available, coreclient.AssistantExtensionBinding{Name: descriptor.Name, Version: descriptor.Version, SchemaSHA256: descriptor.SchemaSHA256, PriceVersion: descriptor.PriceVersion, PriceSHA256: descriptor.PriceSHA256})
		}
		if err := client.AgentExtensionsReady(ctx, available); err != nil {
			return err
		}
		chat, chatErr := assistant.NewAgentChat(authority, r.AssistantPending, agentJournal, agentPort, registry, r.AssistantHistory)
		if chatErr != nil {
			return chatErr
		}
		r.AssistantChat = chat.WithGifts(gifts).WithHumanSupportGuard(personal.Domain())
		return nil
	}
	r.AssistantChat, err = assistant.NewPersonalChat(r.AssistantPending, r.AssistantTurns, r.AssistantHistory, gifts)
	return err
}
