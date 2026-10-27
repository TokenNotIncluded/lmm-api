package webcompat

import (
	"context"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules"
)

type pluginControllerFake struct {
	info     []modules.PluginInfo
	enabled  map[string]bool
	installs int
	reloads  int
}

func (f *pluginControllerFake) List() []modules.PluginInfo {
	return append([]modules.PluginInfo(nil), f.info...)
}
func (f *pluginControllerFake) IsEnabled(name string) bool { return f.enabled[name] }
func (f *pluginControllerFake) Install(request modules.PluginInstallRequest) (modules.PluginInfo, error) {
	f.installs++
	if request.Name != "alpha" {
		return modules.PluginInfo{}, modules.ErrPluginNotFound
	}
	return f.info[0], nil
}
func (f *pluginControllerFake) Enable(name string) (modules.PluginInfo, error) {
	if name != "alpha" {
		return modules.PluginInfo{}, modules.ErrPluginNotFound
	}
	f.enabled[name] = true
	return f.info[0], nil
}
func (f *pluginControllerFake) Disable(name string) (modules.PluginInfo, error) {
	if name != "alpha" {
		return modules.PluginInfo{}, modules.ErrPluginNotFound
	}
	f.enabled[name] = false
	return f.info[0], nil
}
func (f *pluginControllerFake) Uninstall(name string) (modules.PluginInfo, error) {
	if name != "alpha" {
		return modules.PluginInfo{}, modules.ErrPluginNotFound
	}
	f.enabled[name] = false
	return f.info[0], nil
}
func (f *pluginControllerFake) Reload() error { f.reloads++; return nil }

func pluginRequest(handler http.Handler, method, path, body, token string) *httptest.ResponseRecorder {
	r := httptest.NewRequest(method, path, strings.NewReader(body))
	if token != "" {
		r.Header.Set("Authorization", "Bearer "+token)
	}
	w := httptest.NewRecorder()
	handler.ServeHTTP(w, r)
	return w
}

func TestPluginAdminRoutesRequirePlatformAdministratorAndSupportLifecycle(t *testing.T) {
	manager := &pluginControllerFake{info: []modules.PluginInfo{{Name: "alpha", Version: "1.0.0", APIVersion: 1, RegistrationSHA256: modules.RegistrationSHA256("alpha", "1.0.0"), Available: true, Installed: true, Enabled: true}}, enabled: map[string]bool{"alpha": true}}
	host, err := modules.NewHost(nil, []byte(hostToken))
	if err != nil {
		t.Fatal(err)
	}
	if err := host.MarkReady(); err != nil {
		t.Fatal(err)
	}
	core := &authCore{}
	handler := Mount(host, newAuth(t, core), nil, nil, AdditionalServices{Plugins: manager})
	if response := pluginRequest(handler, http.MethodGet, "/api/extensions/plugins", "", ""); response.Code != http.StatusUnauthorized {
		t.Fatalf("anonymous status=%d body=%s", response.Code, response.Body)
	}
	if response := pluginRequest(handler, http.MethodGet, "/api/extensions/plugins", "", webToken); response.Code != http.StatusForbidden {
		t.Fatalf("user status=%d body=%s", response.Code, response.Body)
	}
	core.fn = func(_ context.Context, _ string) (*pb.AuthorizeResponse, error) {
		self := sessionSelf()
		self.PlatformLevel = 5
		return self, nil
	}
	if response := pluginRequest(handler, http.MethodGet, "/api/extensions/plugins", "", webToken); response.Code != http.StatusOK {
		t.Fatalf("admin list status=%d body=%s", response.Code, response.Body)
	}
	manifest := `{"name":"alpha","version":"1.0.0","api_version":1,"registration_sha256":"` + modules.RegistrationSHA256("alpha", "1.0.0") + `"}`
	if response := pluginRequest(handler, http.MethodPost, "/api/extensions/plugins/install", manifest, webToken); response.Code != http.StatusOK || manager.installs != 1 {
		t.Fatalf("install status=%d body=%s installs=%d", response.Code, response.Body, manager.installs)
	}
	if response := pluginRequest(handler, http.MethodPost, "/api/extensions/plugins/reload", "{}", webToken); response.Code != http.StatusUnprocessableEntity {
		t.Fatalf("reload body status=%d body=%s", response.Code, response.Body)
	}
	if response := pluginRequest(handler, http.MethodPost, "/api/extensions/plugins/reload", " ", webToken); response.Code != http.StatusUnprocessableEntity {
		t.Fatalf("reload whitespace status=%d body=%s", response.Code, response.Body)
	}
	if response := pluginRequest(handler, http.MethodPost, "/api/extensions/plugins/reload", "", webToken); response.Code != http.StatusOK || manager.reloads != 1 {
		t.Fatalf("reload status=%d body=%s reloads=%d", response.Code, response.Body, manager.reloads)
	}
	if response := pluginRequest(handler, http.MethodPost, "/api/extensions/plugins/alpha/disable", "", webToken); response.Code != http.StatusOK || manager.enabled["alpha"] {
		t.Fatalf("disable status=%d body=%s", response.Code, response.Body)
	}
	if response := pluginRequest(handler, http.MethodPost, "/api/extensions/plugins/alpha/enable", "", webToken); response.Code != http.StatusOK || !manager.enabled["alpha"] {
		t.Fatalf("enable status=%d body=%s", response.Code, response.Body)
	}
}

func TestPluginInstallRejectsManifestThatExceedsBodyLimitWithWhitespace(t *testing.T) {
	manager := &pluginControllerFake{info: []modules.PluginInfo{{Name: "alpha", Version: "1.0.0", APIVersion: 1, RegistrationSHA256: modules.RegistrationSHA256("alpha", "1.0.0"), Available: true}}, enabled: map[string]bool{}}
	host, err := modules.NewHost(nil, []byte(hostToken))
	if err != nil {
		t.Fatal(err)
	}
	if err := host.MarkReady(); err != nil {
		t.Fatal(err)
	}
	core := &authCore{fn: func(_ context.Context, _ string) (*pb.AuthorizeResponse, error) {
		self := sessionSelf()
		self.PlatformLevel = 5
		return self, nil
	}}
	handler := Mount(host, newAuth(t, core), nil, nil, AdditionalServices{Plugins: manager})
	manifest := `{"name":"alpha","version":"1.0.0","api_version":1,"registration_sha256":"` + modules.RegistrationSHA256("alpha", "1.0.0") + `"}`
	response := pluginRequest(handler, http.MethodPost, "/api/extensions/plugins/install", manifest+strings.Repeat(" ", 16<<10), webToken)
	if response.Code != http.StatusUnprocessableEntity || manager.installs != 0 {
		t.Fatalf("status=%d body=%s installs=%d", response.Code, response.Body, manager.installs)
	}
}

func TestDisabledPluginRejectsWebCompatibilityRouteBeforeService(t *testing.T) {
	manager := &pluginControllerFake{enabled: map[string]bool{"store": false, "support": true}}
	host, err := modules.NewHost(nil, []byte(hostToken))
	if err != nil {
		t.Fatal(err)
	}
	if err := host.MarkReady(); err != nil {
		t.Fatal(err)
	}
	supportService := &supportFake{}
	handler := Mount(host, newAuth(t, &authCore{}), newStoreTransportFake(), supportService, AdditionalServices{Plugins: manager})
	response := pluginRequest(handler, http.MethodGet, "/api/store/products", "", webToken)
	if response.Code != http.StatusServiceUnavailable || !strings.Contains(response.Body.String(), "STORE_UNAVAILABLE") {
		t.Fatalf("status=%d body=%s", response.Code, response.Body)
	}
	for _, enabled := range []map[string]bool{
		{"store": true, "support": false},
		{"store": false, "support": true},
	} {
		manager.enabled = enabled
		response = pluginRequest(handler, http.MethodGet, "/api/store/support/conversations", "", webToken)
		if response.Code != http.StatusServiceUnavailable || !strings.Contains(response.Body.String(), "STORE_UNAVAILABLE") {
			t.Fatalf("enabled=%v status=%d body=%s", enabled, response.Code, response.Body)
		}
	}
	if len(supportService.calls) != 0 {
		t.Fatalf("disabled support reached service: %+v", supportService.calls)
	}
}

var _ PluginController = (*pluginControllerFake)(nil)
