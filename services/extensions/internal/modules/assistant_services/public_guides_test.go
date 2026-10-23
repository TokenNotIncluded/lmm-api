// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later
package assistant_services

import (
	"context"
	"encoding/json"
	"errors"
	"github.com/TokenNotIncluded/api.lmm.best/extensions/internal/modules/assistant/access"
	"strings"
	"testing"
)

type guideAuth struct {
	principal   access.Principal
	err         error
	credentials []string
}

func (a *guideAuth) Check(_ context.Context, credential string, _ access.Account, _ access.Permission) (access.Principal, error) {
	a.credentials = append(a.credentials, credential)
	return a.principal, a.err
}

type guideSnapshots struct {
	snapshot PublicConfigurationSnapshot
	err      error
}

func (s *guideSnapshots) PublicSnapshot(context.Context) (PublicConfigurationSnapshot, error) {
	return s.snapshot, s.err
}

type guideCatalog struct {
	snapshot    ModelCatalogSnapshot
	err         error
	credentials []string
}

func (c *guideCatalog) AvailableModels(_ context.Context, credential string) (ModelCatalogSnapshot, error) {
	c.credentials = append(c.credentials, credential)
	return c.snapshot, c.err
}
func newGuideFixture(t *testing.T) (*PublicGuides, *guideAuth, *guideSnapshots, *guideCatalog) {
	t.Helper()
	a := &guideAuth{principal: access.Principal{UserID: 7, Account: access.Account{Kind: "personal", ID: 7}}}
	s := &guideSnapshots{snapshot: PublicConfigurationSnapshot{Revision: "reviewed-runtime-v1", ServiceRoot: "https://console.example.test/", CheckinEnabled: true, CheckinMinCredits: 10, CheckinMaxCredits: 20, CCSwitchImportEnabled: true}}
	c := &guideCatalog{snapshot: ModelCatalogSnapshot{ActorUserID: 7, DeveloperAccessGranted: true, ModelIDs: []string{"exact/model-A"}, KeyGroupOptions: []PublicKeyGroup{{ID: "reviewed-group", Description: "Reviewed current user group"}}, CheckinUser: &PublicCheckinUser{TrustLevel: 2, Multiplier: 2, MinCredits: 20, MaxCredits: 40, CheckedInToday: true}}}
	g, err := NewPublicGuides(a, s, c)
	if err != nil {
		t.Fatal(err)
	}
	return g, a, s, c
}
func decodeGuide(t *testing.T, raw json.RawMessage) map[string]any {
	t.Helper()
	var result map[string]any
	if json.Unmarshal(raw, &result) != nil {
		t.Fatal(string(raw))
	}
	return result
}
func TestPublicFactsFreshSessionAndConfiguration(t *testing.T) {
	g, a, s, c := newGuideFixture(t)
	raw, err := g.ServiceFacts(context.Background(), "original-session")
	if err != nil {
		t.Fatal(err)
	}
	r := decodeGuide(t, raw)
	if r["service_root"] != "https://console.example.test" || r["openai_base_url"] != "https://console.example.test/v1" || r["group_catalog_source"] != "live_user_usable_groups" {
		t.Fatal(r)
	}
	checkin := r["activities"].(map[string]any)["daily_checkin"].(map[string]any)
	if checkin["enabled"] != true || checkin["min_quota"] != float64(20) || checkin["checked_in_today"] != true {
		t.Fatal(checkin)
	}
	s.snapshot.CheckinEnabled = false
	s.snapshot.CCSwitchImportEnabled = false
	s.snapshot.Revision = "reviewed-runtime-v2"
	raw, err = g.ServiceFacts(context.Background(), "original-session")
	if err != nil {
		t.Fatal(err)
	}
	r = decodeGuide(t, raw)
	checkin = r["activities"].(map[string]any)["daily_checkin"].(map[string]any)
	if checkin["enabled"] != false || checkin["min_quota"] != nil || r["cc_switch_import"].(map[string]any)["supported"] != false || r["configuration_revision"] != "reviewed-runtime-v2" {
		t.Fatal(r)
	}
	if len(a.credentials) != 2 || len(c.credentials) != 2 || a.credentials[0] != "original-session" || c.credentials[1] != "original-session" {
		t.Fatal("original live session was not revalidated")
	}
	a.err = access.ErrUnauthorized
	if _, err := g.ServiceFacts(context.Background(), "revoked"); !errors.Is(err, access.ErrUnauthorized) || len(c.credentials) != 2 {
		t.Fatal(err)
	}
}
func TestPublicGuidesExactModelsAndLockedPreview(t *testing.T) {
	g, _, s, c := newGuideFixture(t)
	for _, tc := range []struct{ args, status string }{{`{"platform":"linux","topic":"codex"}`, "model_required"}, {`{"platform":"linux","topic":"codex","model_id":"model-A"}`, "model_unavailable"}} {
		raw, err := g.SetupGuide(context.Background(), "session", json.RawMessage(tc.args))
		if err != nil || decodeGuide(t, raw)["status"] != tc.status {
			t.Fatal(string(raw), err)
		}
	}
	c.snapshot.DeveloperAccessGranted = false
	c.snapshot.AccountModelAccessLocked = true
	c.snapshot.PublicPreview = true
	raw, err := g.SetupGuide(context.Background(), "session", json.RawMessage(`{"platform":"windows","topic":"codex","model_id":"exact/model-A"}`))
	if err != nil {
		t.Fatal(err)
	}
	r := decodeGuide(t, raw)
	if r["account_model_access_locked"] != true || r["api_key"] != "<YOUR_API_KEY>" || !strings.Contains(r["security_note"].(string), "locked until L1 approval") || !strings.Contains(r["config_toml"].(string), "exact/model-A") {
		t.Fatal(r)
	}
	s.snapshot.CCSwitchImportEnabled = false
	raw, err = g.SetupGuide(context.Background(), "session", json.RawMessage(`{"platform":"linux","topic":"cc-switch","model_id":"exact/model-A"}`))
	if err != nil || decodeGuide(t, raw)["status"] != "client_import_unavailable" {
		t.Fatal(string(raw), err)
	}
}
func TestPublicSetupTopicPlatformSourceMatrix(t *testing.T) {
	g, _, _, _ := newGuideFixture(t)
	for _, topic := range []string{"claude-code", "cc-switch", "claude-desktop", "chatgpt-client", "codex", "cursor", "open-webui", "cherry-studio", "chatbox", "other-openai-compatible"} {
		for _, platform := range []string{"windows", "linux", "macos", "android", "ios"} {
			args, _ := json.Marshal(setupGuideInput{Platform: platform, Topic: topic, ModelID: "exact/model-A"})
			raw, err := g.SetupGuide(context.Background(), "session", args)
			if err != nil {
				t.Fatal(topic, platform, err)
			}
			r := decodeGuide(t, raw)
			if r["ok"] != true || r["client_model_id"] != "exact/model-A" {
				t.Fatal(r)
			}
			if r["supported"] == false && r["verification"] != nil {
				t.Fatal("unsupported client claimed verification", r)
			}
		}
	}
}
func TestPublicGuideRejectsUnboundOrMalformedInputs(t *testing.T) {
	for _, mode := range []string{"other-user", "team", "contradictory-entitlement", "duplicate-model", "unconfigured-root", "secret-url", "invalid-local"} {
		t.Run(mode, func(t *testing.T) {
			g, a, s, c := newGuideFixture(t)
			switch mode {
			case "other-user":
				c.snapshot.ActorUserID = 8
			case "team":
				a.principal.Account = access.Account{Kind: "team", ID: 7}
			case "contradictory-entitlement":
				c.snapshot.AccountModelAccessLocked = true
			case "duplicate-model":
				c.snapshot.ModelIDs = append(c.snapshot.ModelIDs, "exact/model-A")
			case "unconfigured-root":
				s.snapshot.ServiceRoot = ""
			case "secret-url":
				s.snapshot.ServiceRoot = "https://user:secret@example.com"
			case "invalid-local":
				s.snapshot.LocalHTTP = true
			}
			if _, err := g.ServiceFacts(context.Background(), "session"); err == nil {
				t.Fatal(mode)
			}
		})
	}
	g, _, _, _ := newGuideFixture(t)
	for _, raw := range []string{`null`, `{} {}`, `{"platform":"linux","topic":"codex","model_id":"exact/model-A","api_key":"secret"}`} {
		_, err := g.SetupGuide(context.Background(), "session", json.RawMessage(raw))
		if err == nil {
			t.Fatal(raw)
		}
	}
}
func TestPublicSetupShellQuoting(t *testing.T) {
	g, _, _, c := newGuideFixture(t)
	c.snapshot.ModelIDs = []string{"exact/$(touch BAD)'model"}
	for _, platform := range []string{"windows", "linux"} {
		args, _ := json.Marshal(setupGuideInput{Platform: platform, Topic: "claude-code", ModelID: c.snapshot.ModelIDs[0]})
		raw, err := g.SetupGuide(context.Background(), "session", args)
		if err != nil {
			t.Fatal(err)
		}
		configuration := decodeGuide(t, raw)["configuration"].(string)
		if platform == "windows" && !strings.Contains(configuration, "'exact/$(touch BAD)''model'") {
			t.Fatal(configuration)
		}
		if platform == "linux" && !strings.Contains(configuration, `'exact/$(touch BAD)'"'"'model'`) {
			t.Fatal(configuration)
		}
	}
}

// Core's actual catalog may grant a bounded public preview without L1.
func TestPublicGuidesCoreUnlockedPublicPreview(t *testing.T) {
	g, _, _, c := newGuideFixture(t)
	c.snapshot.DeveloperAccessGranted = false
	c.snapshot.AccountModelAccessLocked = false
	c.snapshot.PublicPreview = true
	raw, err := g.SetupGuide(context.Background(), "session", json.RawMessage(`{"platform":"linux","topic":"codex","model_id":"exact/model-A"}`))
	if err != nil {
		t.Fatal(err)
	}
	if decodeGuide(t, raw)["account_model_access_locked"] != false {
		t.Fatalf("Core preview entitlement changed: %s", raw)
	}
}
