// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later
package assistant_services

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"github.com/TokenNotIncluded/lmm-api/services/extensions/internal/modules/assistant/access"
	"io"
	"net/netip"
	"net/url"
	"slices"
	"strings"
	"unicode/utf8"
)

// PublicConfigurationSnapshot is read from the host's actual reviewed runtime
// configuration. Revision must identify that snapshot, not a guessed default.
type PublicConfigurationSnapshot struct {
	Revision              string
	ServiceRoot           string
	LocalHTTP             bool
	CheckinEnabled        bool
	CheckinMinCredits     int64
	CheckinMaxCredits     int64
	CCSwitchImportEnabled bool
}
type PublicSnapshotReader interface {
	PublicSnapshot(context.Context) (PublicConfigurationSnapshot, error)
}
type PublicGroupWarning struct {
	Enabled       bool   `json:"enabled"`
	Message       string `json:"message"`
	Mode          string `json:"mode"`
	Confirmations int    `json:"confirmations"`
}
type PublicKeyGroup struct {
	ID          string              `json:"id"`
	Description string              `json:"description"`
	Warning     *PublicGroupWarning `json:"warning,omitempty"`
}
type PublicCheckinUser struct {
	TrustLevel     int
	Multiplier     int64
	MinCredits     int64
	MaxCredits     int64
	CheckedInToday bool
}

// AvailableModelProjection must use the original session in a fresh Core call.
// ActorUserID binds the response to that session; Go supplies no entitlement.
type ModelCatalogSnapshot struct {
	ActorUserID              int64
	DeveloperAccessGranted   bool
	AccountModelAccessLocked bool
	PublicPreview            bool
	ModelIDs                 []string
	KeyGroupOptions          []PublicKeyGroup
	CheckinUser              *PublicCheckinUser
}
type AvailableModelProjection interface {
	AvailableModels(context.Context, string) (ModelCatalogSnapshot, error)
}
type PublicGuides struct {
	auth      access.Authority
	snapshots PublicSnapshotReader
	catalog   AvailableModelProjection
}

func NewPublicGuides(auth access.Authority, snapshots PublicSnapshotReader, catalog AvailableModelProjection) (*PublicGuides, error) {
	if access.Missing(auth) || access.Missing(snapshots) || access.Missing(catalog) {
		return nil, access.ErrInvalid
	}
	return &PublicGuides{auth, snapshots, catalog}, nil
}
func (g *PublicGuides) current(ctx context.Context, credential string) (PublicConfigurationSnapshot, ModelCatalogSnapshot, error) {
	var empty PublicConfigurationSnapshot
	var noCatalog ModelCatalogSnapshot
	p, err := g.auth.Check(ctx, credential, access.Account{}, access.Read)
	if err != nil {
		return empty, noCatalog, err
	}
	if p.UserID <= 0 || p.Account != (access.Account{Kind: "personal", ID: p.UserID}) {
		return empty, noCatalog, access.ErrForbidden
	}
	snapshot, err := g.snapshots.PublicSnapshot(ctx)
	if err != nil {
		return empty, noCatalog, err
	}
	if !validPublicSnapshot(snapshot) {
		return empty, noCatalog, access.ErrUnavailable
	}
	catalog, err := g.catalog.AvailableModels(ctx, credential)
	if err != nil {
		return empty, noCatalog, err
	}
	if catalog.ActorUserID != p.UserID || (catalog.DeveloperAccessGranted && catalog.AccountModelAccessLocked) || (!catalog.DeveloperAccessGranted && !catalog.AccountModelAccessLocked && !catalog.PublicPreview) || len(catalog.ModelIDs) > 4096 || len(catalog.KeyGroupOptions) > 128 {
		return empty, noCatalog, access.ErrUnavailable
	}
	seen := map[string]bool{}
	for _, id := range catalog.ModelIDs {
		if !validGuideText(id, 256) || seen[id] {
			return empty, noCatalog, access.ErrUnavailable
		}
		seen[id] = true
	}
	for _, group := range catalog.KeyGroupOptions {
		if !validGuideText(group.ID, 128) || !utf8.ValidString(group.Description) || len(group.Description) > 2048 {
			return empty, noCatalog, access.ErrUnavailable
		}
		if group.Warning != nil && (!utf8.ValidString(group.Warning.Message) || len(group.Warning.Message) > 4096 || group.Warning.Confirmations < 0 || group.Warning.Confirmations > 16) {
			return empty, noCatalog, access.ErrUnavailable
		}
	}
	if user := catalog.CheckinUser; user != nil && (user.TrustLevel < 0 || user.TrustLevel > 6 || user.Multiplier < 0 || user.MinCredits < 0 || user.MaxCredits < user.MinCredits) {
		return empty, noCatalog, access.ErrUnavailable
	}
	return snapshot, catalog, nil
}
func validGuideText(s string, max int) bool {
	if !utf8.ValidString(s) || len(s) == 0 || len(s) > max {
		return false
	}
	for _, r := range s {
		if r < 32 || r == 127 {
			return false
		}
	}
	return strings.TrimSpace(s) == s
}
func validPublicSnapshot(s PublicConfigurationSnapshot) bool {
	if !validGuideText(s.Revision, 128) || s.CheckinMinCredits < 0 || s.CheckinMaxCredits < s.CheckinMinCredits {
		return false
	}
	u, err := url.Parse(s.ServiceRoot)
	if err != nil || u.Host == "" || u.User != nil || u.RawQuery != "" || u.Fragment != "" || u.Opaque != "" || len(s.ServiceRoot) > 2048 || strings.ContainsAny(s.ServiceRoot, "\r\n\\") {
		return false
	}
	if s.LocalHTTP {
		ip, e := netip.ParseAddr(u.Hostname())
		return u.Scheme == "http" && e == nil && ip.Unmap().IsLoopback()
	}
	return u.Scheme == "https"
}
func marshalPublicGuide(value any) (json.RawMessage, error) {
	raw, err := json.Marshal(value)
	if err != nil || len(raw) > 64<<10 {
		return nil, access.ErrUnavailable
	}
	return raw, nil
}
func (g *PublicGuides) ServiceFacts(ctx context.Context, credential string) (json.RawMessage, error) {
	snapshot, catalog, err := g.current(ctx, credential)
	if err != nil {
		return nil, err
	}
	root := strings.TrimRight(snapshot.ServiceRoot, "/")
	checkin := map[string]any{"enabled": snapshot.CheckinEnabled, "page_path": "/profile", "status_endpoint": "/api/user/checkin", "frequency": "once_per_day", "reward_type": "random_quota"}
	if snapshot.CheckinEnabled {
		checkin["base_min_quota"] = snapshot.CheckinMinCredits
		checkin["base_max_quota"] = snapshot.CheckinMaxCredits
		if user := catalog.CheckinUser; user != nil {
			checkin["trust_level"] = user.TrustLevel
			checkin["reward_multiplier"] = user.Multiplier
			checkin["min_quota"] = user.MinCredits
			checkin["max_quota"] = user.MaxCredits
			checkin["checked_in_today"] = user.CheckedInToday
		}
	}
	return marshalPublicGuide(map[string]any{"ok": true, "configuration_revision": snapshot.Revision, "service_root": root, "openai_base_url": root + "/v1", "client_model_instruction": "Call get_available_models and use an exact model_ids value; the assistant's own model is not a client default.", "api_keys_are_private": true, "key_management_path": "/keys", "key_group_options": catalog.KeyGroupOptions, "group_catalog_source": "live_user_usable_groups", "activities": map[string]any{"daily_checkin": checkin}, "cc_switch_import": map[string]any{"supported": snapshot.CCSwitchImportEnabled, "protocol": "ccswitch://v1/import", "application": "claude", "requires_private_api_key": true, "ui_action": "Import to CC Switch"}, "write_actions": "require explicit confirmation in the UI"})
}

type setupGuideInput struct {
	Platform string `json:"platform"`
	Topic    string `json:"topic"`
	ModelID  string `json:"model_id"`
}

func (g *PublicGuides) SetupGuide(ctx context.Context, credential string, args json.RawMessage) (json.RawMessage, error) {
	if len(args) == 0 || len(args) > 16<<10 {
		return nil, access.ErrInvalid
	}
	args = bytes.TrimSpace(args)
	if len(args) == 0 || args[0] != '{' {
		return nil, access.ErrInvalid
	}
	var input setupGuideInput
	d := json.NewDecoder(bytes.NewReader(args))
	d.DisallowUnknownFields()
	if d.Decode(&input) != nil {
		return nil, access.ErrInvalid
	}
	var extra any
	if d.Decode(&extra) != io.EOF {
		return nil, access.ErrInvalid
	}
	snapshot, catalog, err := g.current(ctx, credential)
	if err != nil {
		return nil, err
	}
	return marshalPublicGuide(buildSetupGuide(snapshot.ServiceRoot, catalog, input, snapshot.CCSwitchImportEnabled))
}

func quotePOSIXShellLiteral(value string) string {
	return "'" + strings.ReplaceAll(value, "'", "'\"'\"'") + "'"
}

// quotePowerShellLiteral returns a single-quoted PowerShell string. PowerShell
// represents a literal apostrophe inside such a string as two apostrophes.
func quotePowerShellLiteral(value string) string {
	return "'" + strings.ReplaceAll(value, "'", "''") + "'"
}

func buildSetupGuide(rootURL string, catalog ModelCatalogSnapshot, input setupGuideInput, ccSwitchEnabled bool) map[string]any {
	platform := strings.ToLower(strings.TrimSpace(input.Platform))
	topic := strings.ToLower(strings.TrimSpace(input.Topic))
	if platform != "windows" && platform != "linux" && platform != "macos" && platform != "android" && platform != "ios" {
		return map[string]any{"ok": false, "error": "platform must be windows, linux, macos, android, or ios"}
	}
	if topic != "claude-code" && topic != "cc-switch" && topic != "claude-desktop" && topic != "chatgpt-client" && topic != "codex" && topic != "cursor" && topic != "open-webui" && topic != "cherry-studio" && topic != "chatbox" && topic != "other-openai-compatible" {
		return map[string]any{"ok": false, "error": "topic is not supported"}
	}
	rootURL = strings.TrimRight(rootURL, "/")
	if rootURL == "" {
		rootURL = "<SERVICE_ROOT_URL>"
	}
	openAIBaseURL := rootURL + "/v1"
	if rootURL == "<SERVICE_ROOT_URL>" {
		openAIBaseURL = "<OPENAI_BASE_URL>"
	}
	clientModel := strings.TrimSpace(input.ModelID)
	if clientModel == "" {
		return map[string]any{
			"ok":        false,
			"status":    "model_required",
			"error":     "an exact model ID is required",
			"next_step": "Call get_available_models and use one exact model_ids value.",
		}
	}
	modelIDs := catalog.ModelIDs
	if !slices.Contains(modelIDs, clientModel) {
		status := "model_unavailable"
		if catalog.PublicPreview {
			status = "model_not_in_public_preview"
		}
		return map[string]any{"ok": false, "status": status, "error": "the requested model ID is not available in the signed-in account catalog", "available_model_ids": modelIDs, "next_step": "Use one exact available_model_ids value; do not guess or rewrite the model ID."}
	}
	accountModelAccessLocked := catalog.AccountModelAccessLocked
	developerAccessGranted := catalog.DeveloperAccessGranted
	if (topic == "cc-switch" || topic == "claude-desktop") && !ccSwitchEnabled {
		return map[string]any{"ok": false, "status": "client_import_unavailable", "error": "CC Switch integration is not enabled by the current host configuration"}
	}
	securityNote := "Create the key in this console, never paste an existing secret into chat, and test with a newly opened terminal or client session."
	credentialStep := "Create an API key in this console and replace only the <YOUR_API_KEY> placeholder."
	credentialPhrase := "a newly created API key"
	testStep := "Send a short test request after configuring the client."
	if accountModelAccessLocked {
		securityNote = "API key creation and authenticated requests remain locked until L1 approval. You can install the client and review the placeholder configuration now without creating or sharing a key."
		credentialStep = "Keep the <YOUR_API_KEY> placeholder while access is locked; after L1 approval, create a key in this console and replace only that placeholder."
		credentialPhrase = "the <YOUR_API_KEY> placeholder (replace it with a new key only after L1 approval)"
		testStep = "After L1 approval and key creation, open a new client session and send a short test request."
	}
	lockedAwareStep := func(unlocked, locked string) string {
		if accountModelAccessLocked {
			return locked
		}
		return unlocked
	}

	result := map[string]any{
		"ok":                          true,
		"platform":                    platform,
		"topic":                       topic,
		"service_root":                rootURL,
		"openai_base_url":             openAIBaseURL,
		"client_model_id":             clientModel,
		"api_key":                     "<YOUR_API_KEY>",
		"developer_access_granted":    developerAccessGranted,
		"account_model_access_locked": accountModelAccessLocked,
		"security_note":               securityNote,
		"verification":                lockedAwareStep("Save the provider, select the exact returned model, and send: Reply with OK. A response without an error confirms this request worked; importing settings alone does not verify connectivity.", testStep),
		"troubleshooting": map[string]string{
			"401": "In the private API-key page, check that the key is enabled, unexpired, and copied without whitespace. Paste it only into the client's API Key field; never into chat.",
			"404": "Check the client-specific host and path below for a missing or duplicate /v1. Use one exact live model ID and a model that supports the client's API route.",
			"429": "Read the error detail to distinguish rate limiting from insufficient quota. Respect Retry-After, reduce concurrent requests, and check account/key quota in the console; do not repeatedly retry or assume a payment is required.",
		},
		"support_details": "If the test fails, share only the client name/version, status code, and redacted error text. Remove API keys, Authorization headers, and key-bearing import links from any screenshot.",
	}
	if (platform == "android" || platform == "ios") && topic != "chatbox" && topic != "chatgpt-client" && topic != "open-webui" && topic != "other-openai-compatible" {
		result["supported"] = false
		result["limitation"] = "This guide is for a desktop client. Use Chatbox on Android or iOS, or continue this client's setup on Windows, macOS, or Linux."
		result["recommended_alternatives"] = []string{"Chatbox"}
		result["official_download"] = "https://chatboxai.app/en/guide/getting-started/download"
		delete(result, "verification")
		return result
	}

	switch topic {
	case "chatbox", "cherry-studio":
		result["supported"] = true
		result["client_api_host"] = rootURL
		result["api_path"] = "/v1/chat/completions"
		result["endpoint_format"] = "OpenAI Chat Completions; API Host is the service root, with /v1/chat/completions as a separate request path. Do not duplicate /v1."
		if topic == "chatbox" {
			result["official_download"] = "https://chatboxai.app/en/guide/getting-started/download"
			result["official_docs"] = "https://docs.chatboxai.app/guides/providers"
			result["steps"] = []string{
				"Open the official Chatbox download page and select " + platform + ". On Android or iOS follow its official store/download link, install the app, then open it.",
				"Open Settings > Model Providers > Add. Name the provider LMM and choose OpenAI API Compatible.",
				"Set API Host to " + rootURL + ". Leave API Path at /v1/chat/completions; do not add /v1 to API Host.",
				credentialStep + " Paste the key only into Chatbox's API Key field.",
				"Add a model with the exact ID " + clientModel + ", then save the provider. Enable only capabilities supported by that model.",
				lockedAwareStep("Click Check and confirm a successful connection. Return to the home screen, start a new chat, select LMM and the configured model, then send: Reply with OK.", testStep),
			}
		} else {
			result["official_download"] = "https://www.cherry-ai.com/download"
			result["official_docs"] = "https://docs.cherryai.com.cn/pre-basic/providers/zi-ding-yi-fu-wu-shang"
			result["steps"] = []string{
				"Open the official Cherry Studio download page, select " + platform + ", install the package for your device, and launch the app.",
				"Open Settings > Model Services > Add Provider. Name it LMM_API and configure an OpenAI-compatible endpoint.",
				"Set API Address to " + rootURL + ". The OpenAI-compatible endpoint adds the API path automatically; do not enter /chat/completions in this field.",
				credentialStep + " Paste the key only into Cherry Studio's API Key field.",
				"Use Manage to fetch models, or Add to enter the exact ID " + clientModel + ". Turn on the provider's enable switch.",
				lockedAwareStep("Click Check with the configured model. Open a new conversation, choose this provider and model, and send: Reply with OK.", testStep),
			}
		}
	case "claude-code":
		installCommand := "curl -fsSL https://claude.ai/install.sh | bash"
		configuration := fmt.Sprintf("export ANTHROPIC_BASE_URL=%s\nexport ANTHROPIC_AUTH_TOKEN='<YOUR_API_KEY>'\nexport ANTHROPIC_MODEL=%s\nclaude", quotePOSIXShellLiteral(rootURL), quotePOSIXShellLiteral(clientModel))
		if platform == "windows" {
			installCommand = "winget install Anthropic.ClaudeCode"
			configuration = fmt.Sprintf("$env:ANTHROPIC_BASE_URL=%s\n$env:ANTHROPIC_AUTH_TOKEN='<YOUR_API_KEY>'\n$env:ANTHROPIC_MODEL=%s\nclaude", quotePowerShellLiteral(rootURL), quotePowerShellLiteral(clientModel))
		} else if platform == "macos" {
			installCommand = "brew install --cask claude-code"
		}
		result["install_command"] = installCommand
		result["configuration"] = configuration
		result["endpoint_format"] = "Anthropic Messages; use the service root without /v1"
		result["steps"] = []string{
			"Install Claude Code with the command returned by this tool, then run claude --version.",
			credentialStep,
			lockedAwareStep("Apply the returned environment variables in a terminal opened for the project, then run claude.", testStep),
		}
		result["official_docs"] = "https://code.claude.com/docs/en/setup"
	case "cc-switch":
		installGuide := "Download CC-Switch-v{version}-Windows.msi from the official GitHub Releases page."
		if platform == "macos" {
			installGuide = "brew install --cask cc-switch"
		} else if platform == "linux" {
			installGuide = "Download the official AppImage or distribution package; on Arch Linux use paru -S cc-switch-bin."
		}
		result["install_guide"] = installGuide
		result["provider"] = map[string]any{
			"application": "Claude",
			"env": map[string]string{
				"ANTHROPIC_BASE_URL":   rootURL,
				"ANTHROPIC_AUTH_TOKEN": "<YOUR_API_KEY>",
				"ANTHROPIC_MODEL":      clientModel,
			},
		}
		result["endpoint_format"] = "Anthropic Messages; use the service root without /v1"
		result["steps"] = []string{
			"Install CC Switch only from its official site or GitHub Releases.",
			"Create or select an API key in this console; the key stays in a shielded private card.",
			"Use Import to CC Switch from that private card (or the key's CC Switch action on /keys). The UI constructs the ccswitch:// link and CC Switch shows an import confirmation.",
			"Select Claude, add a Custom provider, and enter the returned service root, model ID, and " + credentialPhrase + ".",
			lockedAwareStep("Confirm the import or save and enable the provider, then open a new terminal and send a short test with Claude Code.", testStep),
		}
		result["cc_switch_import"] = map[string]any{
			"supported":   true,
			"protocol":    "ccswitch://v1/import",
			"resource":    "provider",
			"application": "claude",
			"endpoint":    rootURL,
			"model":       clientModel,
			"api_key":     "<PRIVATE_API_KEY>",
			"link_parameters": map[string]any{
				"resource": "provider",
				"app":      "claude",
				"name":     "LMM",
				"endpoint": rootURL,
				"apiKey":   "<PRIVATE_API_KEY>",
				"model":    clientModel,
				"homepage": rootURL,
				"enabled":  true,
			},
			"build_instructions": "After the user confirms and creates a key, the assistant UI replaces <PRIVATE_API_KEY> client-side and opens the CC Switch import confirmation. Never print the completed URL or ask the user to paste the key into chat.",
		}
		result["official_releases"] = "https://github.com/farion1231/cc-switch/releases"
		result["official_docs"] = "https://github.com/farion1231/cc-switch"
	case "claude-desktop":
		result["direct_custom_gateway_supported"] = false
		result["endpoint_format"] = "Anthropic Messages through CC Switch local routing"
		if platform == "linux" {
			result["supported"] = false
			result["limitation"] = "CC Switch currently manages third-party Claude Desktop profiles on Windows and macOS; use Claude Code on Linux for this service."
		} else {
			result["supported"] = true
			result["steps"] = []string{
				"Install and launch the official Claude Desktop app once.",
				"In CC Switch, enable Claude Desktop and import the Claude Code provider or add a custom provider.",
				"Map the Sonnet role to the returned model ID, enable local routing, then fully restart Claude Desktop.",
			}
		}
		result["official_docs"] = "https://code.claude.com/docs/en/desktop-quickstart"
		result["cc_switch_docs"] = "https://github.com/farion1231/cc-switch/blob/main/docs/user-manual/en/2-providers/2.6-claude-desktop.md"
	case "chatgpt-client":
		result["supported"] = false
		result["direct_custom_gateway_supported"] = false
		result["limitation"] = "The official ChatGPT app uses OpenAI sign-in and does not accept this service's Base URL or API key as a custom provider."
		result["recommended_alternatives"] = []string{"Chatbox", "Cherry Studio (Windows, macOS, Linux)", "CC Switch (desktop coding tools)"}
		result["official_download"] = "https://chatgpt.com/download/"
	case "codex":
		apiKeyCommand := "export LMM_API_KEY='<YOUR_API_KEY>'"
		if platform == "windows" {
			apiKeyCommand = "$env:LMM_API_KEY='<YOUR_API_KEY>'"
		}
		result["install_command"] = "npm install -g @openai/codex"
		result["api_key_command"] = apiKeyCommand
		result["config_path"] = "~/.codex/config.toml"
		result["config_toml"] = fmt.Sprintf("model = %q\nmodel_provider = \"lmm\"\n\n[model_providers.lmm]\nname = \"LMM\"\nbase_url = %q\nenv_key = \"LMM_API_KEY\"\nwire_api = \"responses\"", clientModel, openAIBaseURL)
		result["endpoint_format"] = "OpenAI Responses API; use the /v1 Base URL"
		result["steps"] = []string{
			"Install Codex, then create the user-level ~/.codex/config.toml with the returned provider configuration.",
			lockedAwareStep("Set LMM_API_KEY in the current shell without writing the key into config.toml.", credentialStep),
			lockedAwareStep("Run codex in a project directory and verify the provider and model shown by /status.", testStep),
		}
		result["official_docs"] = "https://developers.openai.com/codex/cli"
		result["config_reference"] = "https://developers.openai.com/codex/config-reference"
	case "cursor":
		result["endpoint_format"] = "OpenAI-compatible; use the /v1 Base URL only if the installed Cursor version exposes a custom Base URL"
		result["steps"] = []string{
			"Open Cursor Settings and check whether the installed version exposes a custom OpenAI-compatible Base URL.",
			"If supported, enter the returned /v1 Base URL, exact model ID, and " + credentialPhrase + ".",
			"If the setting is absent, do not assume the official client can use this gateway; choose CC Switch or another compatible client.",
		}
	case "open-webui":
		result["endpoint_format"] = "OpenAI-compatible; use the /v1 Base URL"
		result["steps"] = []string{
			"Open Open WebUI administrator settings and add an OpenAI-compatible connection.",
			"Enter the returned /v1 Base URL and " + credentialPhrase + ", then refresh the model list.",
			lockedAwareStep("Select the exact returned model ID and send a short test request.", testStep),
		}
		result["official_docs"] = "https://docs.openwebui.com/getting-started/quick-start/connect-a-provider/starting-with-openai-compatible/"
	case "other-openai-compatible":
		result["endpoint_format"] = "OpenAI-compatible; use the /v1 Base URL"
		result["steps"] = []string{
			"Confirm that the client explicitly supports a custom OpenAI-compatible Base URL.",
			"Enter the returned /v1 Base URL, exact model ID, and " + credentialPhrase + ".",
			lockedAwareStep("Send a short test and verify that the client uses a route supported by this service.", testStep),
		}
	}
	if result["supported"] == false {
		delete(result, "verification")
	}
	return result
}
