package coreclient

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"sort"
	"strings"

	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
)

type AssistantExtensionBinding struct{ Name, Version, SchemaSHA256, PriceVersion, PriceSHA256 string }

func extensionTuple(b AssistantExtensionBinding) [5]string {
	return [5]string{b.Name, b.Version, b.SchemaSHA256, b.PriceVersion, b.PriceSHA256}
}
func extensionDigest(v any) string {
	raw, _ := json.Marshal(v)
	h := sha256.Sum256(raw)
	return hex.EncodeToString(h[:])
}

// AgentExtensionsReady checks the Core host's explicit enabled subset against
// installed, dependency-ready Go code. Go definitions never register authority.
func (c *AssistantClient) AgentExtensionsReady(ctx context.Context, available []AssistantExtensionBinding) error {
	if err := c.AgentReady(ctx); err != nil {
		return err
	}
	capabilities, err := c.control.Capabilities(ctx)
	if err != nil || !assistantMessage(capabilities) || len(capabilities.GetFeatures()) > 128 || capabilities.GetProtocolMajor() != 1 {
		return agentExtensionsUnavailable()
	}
	return validateAgentExtensions(capabilities.GetFeatures(), available)
}
func validateAgentExtensions(features []string, available []AssistantExtensionBinding) error {
	if len(features) > 128 || len(available) > 72 {
		return agentExtensionsUnavailable()
	}
	bindings := map[string]AssistantExtensionBinding{}
	for _, b := range available {
		if bindings[b.Name].Name != "" || !agentIdentifier.MatchString(b.Name) || len(b.Name) > 80 || !assistantDigest(b.SchemaSHA256, "") || !assistantDigest(b.PriceSHA256, "") {
			return agentExtensionsUnavailable()
		}
		bindings[b.Name] = b
	}
	const prefix = "assistant.agent_extension.v1:"
	const aggregatePrefix = "assistant.agent_extensions.sha256:"
	seen := map[string]bool{}
	expected := map[string]AssistantExtensionBinding{}
	aggregate := ""
	hasAgent := false
	for _, feature := range features {
		if !assistantLabel(feature, 256) || seen[feature] {
			return agentExtensionsUnavailable()
		}
		seen[feature] = true
		if feature == AssistantAgentTurnFeature {
			hasAgent = true
		}
		if strings.HasPrefix(feature, aggregatePrefix) {
			if aggregate != "" {
				return agentExtensionsUnavailable()
			}
			aggregate = strings.TrimPrefix(feature, aggregatePrefix)
			if !assistantDigest(aggregate, "") {
				return agentExtensionsUnavailable()
			}
		}
		if strings.HasPrefix(feature, prefix) {
			parts := strings.Split(strings.TrimPrefix(feature, prefix), ":")
			if len(parts) != 2 || expected[parts[0]].Name != "" {
				return agentExtensionsUnavailable()
			}
			binding, ok := bindings[parts[0]]
			if !ok || extensionDigest(extensionTuple(binding)) != parts[1] {
				return agentExtensionsUnavailable()
			}
			expected[parts[0]] = binding
		}
	}
	if !hasAgent || aggregate == "" {
		return agentExtensionsUnavailable()
	}
	names := make([]string, 0, len(expected))
	for name := range expected {
		names = append(names, name)
	}
	sort.Strings(names)
	tuples := make([][5]string, 0, len(names))
	for _, name := range names {
		tuples = append(tuples, extensionTuple(expected[name]))
	}
	if extensionDigest(tuples) != aggregate {
		return agentExtensionsUnavailable()
	}
	return nil
}
func agentExtensionsUnavailable() error {
	return status.Error(codes.FailedPrecondition, "assistant extension capabilities unavailable")
}
