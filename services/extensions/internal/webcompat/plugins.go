package webcompat

import (
	"bytes"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"strings"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
)

// PluginController is the small boundary exposed to the Web compatibility
// layer. The implementation owns compiled registrations, route snapshots and
// private lifecycle state; handlers never receive a database or host token.
type PluginController interface {
	List() []modules.PluginInfo
	IsEnabled(string) bool
	Install(modules.PluginInstallRequest) (modules.PluginInfo, error)
	Enable(string) (modules.PluginInfo, error)
	Disable(string) (modules.PluginInfo, error)
	Uninstall(string) (modules.PluginInfo, error)
	Reload() error
}

// RegisterPlugins exposes administrator-only lifecycle operations. Installation
// accepts a manifest for a compiled registration; executable code is shipped in
// a reviewed extension release and is never loaded from an HTTP request.
func RegisterPlugins(mux *http.ServeMux, auth *Authenticator, manager PluginController) {
	if mux == nil || auth == nil || manager == nil {
		return
	}
	admin := func(fn func(http.ResponseWriter, *http.Request)) http.HandlerFunc {
		return func(w http.ResponseWriter, r *http.Request) {
			actor, err := auth.Verify(r.Context(), r)
			if err != nil {
				pluginRespond(w, nil, err)
				return
			}
			if actor.GetPlatformLevel() != 5 && actor.GetPlatformLevel() != 6 {
				pluginRespond(w, nil, store.ErrForbidden)
				return
			}
			fn(w, r)
		}
	}
	mux.HandleFunc("GET /api/extensions/plugins", admin(func(w http.ResponseWriter, _ *http.Request) {
		pluginRespond(w, manager.List(), nil)
	}))
	mux.HandleFunc("POST /api/extensions/plugins/install", admin(func(w http.ResponseWriter, r *http.Request) {
		request, err := decodePluginInstall(r)
		if err != nil {
			pluginRespond(w, nil, modules.ErrPluginManifest)
			return
		}
		info, err := manager.Install(request)
		pluginRespond(w, info, err)
	}))
	mux.HandleFunc("POST /api/extensions/plugins/reload", admin(func(w http.ResponseWriter, r *http.Request) {
		if !emptyPluginBody(r) {
			pluginRespond(w, nil, modules.ErrPluginManifest)
			return
		}
		pluginRespond(w, nil, manager.Reload())
	}))
	mux.HandleFunc("POST /api/extensions/plugins/{name}/enable", admin(func(w http.ResponseWriter, r *http.Request) {
		if !emptyPluginBody(r) {
			pluginRespond(w, nil, modules.ErrPluginManifest)
			return
		}
		info, err := manager.Enable(r.PathValue("name"))
		pluginRespond(w, info, err)
	}))
	mux.HandleFunc("POST /api/extensions/plugins/{name}/disable", admin(func(w http.ResponseWriter, r *http.Request) {
		if !emptyPluginBody(r) {
			pluginRespond(w, nil, modules.ErrPluginManifest)
			return
		}
		info, err := manager.Disable(r.PathValue("name"))
		pluginRespond(w, info, err)
	}))
	mux.HandleFunc("DELETE /api/extensions/plugins/{name}", admin(func(w http.ResponseWriter, r *http.Request) {
		if !emptyPluginBody(r) {
			pluginRespond(w, nil, modules.ErrPluginManifest)
			return
		}
		info, err := manager.Uninstall(r.PathValue("name"))
		pluginRespond(w, info, err)
	}))
}

func decodePluginInstall(r *http.Request) (modules.PluginInstallRequest, error) {
	if r == nil || r.Body == nil {
		return modules.PluginInstallRequest{}, modules.ErrPluginManifest
	}
	// Read one byte beyond the limit so trailing whitespace cannot make an
	// oversized manifest look like a valid, fully consumed JSON document.
	const maxManifestBytes = 16 << 10
	payload, err := io.ReadAll(io.LimitReader(r.Body, maxManifestBytes+1))
	if err != nil || len(payload) > maxManifestBytes {
		return modules.PluginInstallRequest{}, modules.ErrPluginManifest
	}
	decoder := json.NewDecoder(bytes.NewReader(payload))
	decoder.DisallowUnknownFields()
	var request modules.PluginInstallRequest
	if err := decoder.Decode(&request); err != nil {
		return modules.PluginInstallRequest{}, modules.ErrPluginManifest
	}
	var extra any
	if err := decoder.Decode(&extra); !errors.Is(err, io.EOF) {
		return modules.PluginInstallRequest{}, modules.ErrPluginManifest
	}
	if request.Name == "" || request.Version == "" || request.APIVersion == 0 || request.RegistrationSHA256 == "" {
		return modules.PluginInstallRequest{}, modules.ErrPluginManifest
	}
	return request, nil
}

func emptyPluginBody(r *http.Request) bool {
	if r == nil || r.Body == nil {
		return true
	}
	// A one-byte read is enough to distinguish a truly empty request from
	// whitespace or any other body. Do not let a truncated body look like EOF.
	payload, err := io.ReadAll(io.LimitReader(r.Body, 1))
	return err == nil && len(payload) == 0
}

func pluginRespond(w http.ResponseWriter, value any, err error) {
	code := http.StatusOK
	body := map[string]any{"success": true, "data": value}
	if err != nil {
		code = http.StatusServiceUnavailable
		label, message := "PLUGIN_UNAVAILABLE", "The extension plugin operation is unavailable."
		switch {
		case errors.Is(err, store.ErrUnauthorized):
			code, label, message = http.StatusUnauthorized, "PLUGIN_LOGIN_REQUIRED", "Sign in to manage plugins."
		case errors.Is(err, store.ErrForbidden):
			code, label, message = http.StatusForbidden, "PLUGIN_ACCESS_DENIED", "Plugin administration requires a platform administrator."
		case errors.Is(err, modules.ErrPluginManifest):
			code, label, message = http.StatusUnprocessableEntity, "PLUGIN_INVALID_MANIFEST", "The plugin manifest does not match a reviewed registration."
		case errors.Is(err, modules.ErrPluginNotFound):
			code, label, message = http.StatusNotFound, "PLUGIN_NOT_FOUND", "The plugin is not registered in this release."
		case errors.Is(err, modules.ErrPluginNotInstalled):
			code, label, message = http.StatusConflict, "PLUGIN_NOT_INSTALLED", "Install the plugin before enabling it."
		case errors.Is(err, modules.ErrPluginAlreadyEnabled), errors.Is(err, modules.ErrPluginAlreadyOff):
			code, label, message = http.StatusConflict, "PLUGIN_STATE_UNCHANGED", "The plugin already has that state."
		case strings.Contains(err.Error(), "disable plugin before uninstall"):
			code, label, message = http.StatusConflict, "PLUGIN_ENABLED", "Disable the plugin before uninstalling it."
		}
		body = map[string]any{"success": false, "code": label, "message": message}
	}
	encoded, encodeErr := json.Marshal(body)
	if encodeErr != nil {
		code = http.StatusServiceUnavailable
		encoded = []byte(`{"success":false,"code":"PLUGIN_UNAVAILABLE","message":"The extension plugin operation is unavailable."}`)
	}
	w.Header().Set("Content-Type", "application/json; charset=utf-8")
	w.Header().Set("Cache-Control", "no-store")
	w.Header().Set("X-Content-Type-Options", "nosniff")
	w.WriteHeader(code)
	_, _ = w.Write(append(encoded, '\n'))
}

func pluginHTTPPath(path string) bool {
	return path == "/api/extensions/plugins" || strings.HasPrefix(path, "/api/extensions/plugins/")
}

func webPluginName(path string) string {
	switch {
	case path == "/mcp/market" || path == "/mcp/market/" || path == "/api/tool-market" || strings.HasPrefix(path, "/api/tool-market/"):
		return "toolmarket"
	case strings.HasPrefix(path, "/api/store/") || path == "/api/store" || strings.HasPrefix(path, "/api/user/auth/store-claim/"):
		return "store"
	case strings.HasPrefix(path, "/api/assistant/") || path == "/api/assistant":
		return "assistant"
	case strings.HasPrefix(path, "/api/user/payments/"):
		return "payments"
	case strings.HasPrefix(path, "/api/release-notes/") || strings.HasPrefix(path, "/api/scripts") || strings.HasPrefix(path, "/scripts"):
		return "operations"
	default:
		return ""
	}
}
