package coreclient

import (
	"strings"
	"testing"
)

func TestAgentExtensionSubsetNegotiation(t *testing.T) {
	b := AssistantExtensionBinding{Name: "remember_memory", Version: "main-3948a0a1", SchemaSHA256: strings.Repeat("a", 64), PriceVersion: "builtin-free-v1", PriceSHA256: strings.Repeat("b", 64)}
	feature := "assistant.agent_extension.v1:" + b.Name + ":" + extensionDigest(extensionTuple(b))
	aggregate := "assistant.agent_extensions.sha256:" + extensionDigest([][5]string{extensionTuple(b)})
	features := []string{AssistantAgentTurnFeature, feature, aggregate}
	if err := validateAgentExtensions(features, []AssistantExtensionBinding{b}); err != nil {
		t.Fatal(err)
	}
	for name, values := range map[string][]string{"missing aggregate": features[:2], "duplicate": append(append([]string{}, features...), feature), "bad aggregate": {AssistantAgentTurnFeature, feature, "assistant.agent_extensions.sha256:" + strings.Repeat("c", 64)}, "bad tuple": {AssistantAgentTurnFeature, strings.Replace(feature, extensionDigest(extensionTuple(b)), strings.Repeat("d", 64), 1), aggregate}} {
		t.Run(name, func(t *testing.T) {
			if validateAgentExtensions(values, []AssistantExtensionBinding{b}) == nil {
				t.Fatal("accepted invalid feature set")
			}
		})
	}
	if validateAgentExtensions(features, nil) == nil {
		t.Fatal("accepted unavailable required adapter")
	}
	changed := b
	changed.SchemaSHA256 = strings.Repeat("d", 64)
	if validateAgentExtensions(features, []AssistantExtensionBinding{changed}) == nil {
		t.Fatal("accepted changed schema")
	}
	// Core-only hosts still commit to an explicit empty extension subset.
	if err := validateAgentExtensions([]string{AssistantAgentTurnFeature, "assistant.agent_extensions.sha256:" + extensionDigest(make([][5]string, 0))}, nil); err != nil {
		t.Fatal(err)
	}
}
