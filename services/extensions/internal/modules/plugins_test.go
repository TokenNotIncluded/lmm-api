package modules

import (
	"errors"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
)

type pluginTestModule struct{ name, version string }

func (m pluginTestModule) Name() string    { return m.name }
func (m pluginTestModule) Version() string { return m.version }
func (m pluginTestModule) Handler() http.Handler {
	return http.HandlerFunc(func(http.ResponseWriter, *http.Request) {})
}

type pluginTestReloader struct {
	last  []string
	calls int
	err   error
}

func (r *pluginTestReloader) ReloadModules(names []string) error {
	r.calls++
	r.last = append([]string(nil), names...)
	return r.err
}

func TestPluginManagerLifecyclePersistsAndHotReloads(t *testing.T) {
	dir := t.TempDir()
	if err := os.Chmod(dir, 0700); err != nil {
		t.Fatal(err)
	}
	state := filepath.Join(dir, "plugins.json")
	reloader := &pluginTestReloader{}
	manager, err := NewPluginManager(reloader, []Module{
		pluginTestModule{"zeta", "2.1.0"}, pluginTestModule{"alpha", "1.0.0"},
	}, state)
	if err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(reloader.last, []string{"alpha", "zeta"}) {
		t.Fatalf("initial reload=%v", reloader.last)
	}
	info := manager.List()
	if len(info) != 2 || info[0].Name != "alpha" || !info[0].Installed || !info[0].Enabled {
		t.Fatalf("inventory=%v", info)
	}
	if _, err := manager.Disable("alpha"); err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(reloader.last, []string{"zeta"}) {
		t.Fatalf("disable reload=%v", reloader.last)
	}
	if _, err := manager.Disable("alpha"); !errors.Is(err, ErrPluginAlreadyOff) {
		t.Fatalf("second disable err=%v", err)
	}
	if _, err := manager.Uninstall("alpha"); err != nil {
		t.Fatal(err)
	}
	if _, err := manager.Enable("alpha"); !errors.Is(err, ErrPluginNotInstalled) {
		t.Fatalf("enable uninstalled err=%v", err)
	}
	alpha := manager.List()[0]
	if _, err := manager.Install(PluginInstallRequest{Name: alpha.Name, Version: alpha.Version, APIVersion: alpha.APIVersion, RegistrationSHA256: alpha.RegistrationSHA256}); err != nil {
		t.Fatal(err)
	}
	if _, err := manager.Enable("alpha"); err != nil {
		t.Fatal(err)
	}
	reloaded, err := NewPluginManager(&pluginTestReloader{}, []Module{
		pluginTestModule{"zeta", "2.1.0"}, pluginTestModule{"alpha", "1.0.0"},
	}, state)
	if err != nil {
		t.Fatal(err)
	}
	for _, got := range reloaded.List() {
		if got.Name == "alpha" && (!got.Installed || !got.Enabled) {
			t.Fatalf("persisted alpha=%+v", got)
		}
	}
}

func TestPluginManagerRejectsUnknownOrMismatchedInstall(t *testing.T) {
	reloader := &pluginTestReloader{}
	manager, err := NewPluginManager(reloader, []Module{pluginTestModule{"alpha", "1.0.0"}}, "")
	if err != nil {
		t.Fatal(err)
	}
	for _, request := range []PluginInstallRequest{
		{Name: "missing", Version: "1.0.0", APIVersion: 1, RegistrationSHA256: strings.Repeat("a", 64)},
		{Name: "alpha", Version: "2.0.0", APIVersion: 1, RegistrationSHA256: RegistrationSHA256("alpha", "1.0.0")},
		{Name: "alpha", Version: "1.0.0", APIVersion: 2, RegistrationSHA256: RegistrationSHA256("alpha", "1.0.0")},
	} {
		if _, err := manager.Install(request); !errors.Is(err, ErrPluginNotFound) && !errors.Is(err, ErrPluginManifest) {
			t.Fatalf("request=%+v err=%v", request, err)
		}
	}
}

func TestPluginManagerInstallPersistenceFailurePreservesStateAndCanRetry(t *testing.T) {
	dir := t.TempDir()
	if err := os.Chmod(dir, 0700); err != nil {
		t.Fatal(err)
	}
	statePath := filepath.Join(dir, "plugins.json")
	manager, err := NewPluginManager(&pluginTestReloader{}, []Module{
		pluginTestModule{"alpha", "1.0.0"},
	}, statePath)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := manager.Disable("alpha"); err != nil {
		t.Fatal(err)
	}
	if _, err := manager.Uninstall("alpha"); err != nil {
		t.Fatal(err)
	}

	before := manager.List()
	if len(before) != 1 || before[0].Installed || before[0].Enabled || manager.IsEnabled("alpha") {
		t.Fatalf("unexpected initial state: list=%+v enabled=%t", before, manager.IsEnabled("alpha"))
	}
	persisted, err := os.ReadFile(statePath)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.Remove(statePath); err != nil {
		t.Fatal(err)
	}
	if err := os.Mkdir(statePath, 0700); err != nil {
		t.Fatal(err)
	}

	request := PluginInstallRequest{
		Name: "alpha", Version: before[0].Version, APIVersion: before[0].APIVersion,
		RegistrationSHA256: before[0].RegistrationSHA256,
	}
	for attempt := 1; attempt <= 2; attempt++ {
		if _, err := manager.Install(request); err == nil {
			t.Fatalf("attempt %d unexpectedly succeeded with an invalid state path", attempt)
		}
		if got := manager.List(); !reflect.DeepEqual(got, before) {
			t.Fatalf("attempt %d changed plugin state: got=%+v want=%+v", attempt, got, before)
		}
		if manager.IsEnabled("alpha") {
			t.Fatalf("attempt %d changed enabled state", attempt)
		}
	}

	if err := os.Remove(statePath); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(statePath, persisted, 0600); err != nil {
		t.Fatal(err)
	}
	installed, err := manager.Install(request)
	if err != nil {
		t.Fatalf("retry after restoring state file: %v", err)
	}
	if !installed.Installed || installed.Enabled || manager.IsEnabled("alpha") {
		t.Fatalf("unexpected successful retry result: %+v", installed)
	}

	reloaded, err := NewPluginManager(&pluginTestReloader{}, []Module{
		pluginTestModule{"alpha", "1.0.0"},
	}, statePath)
	if err != nil {
		t.Fatal(err)
	}
	if got := reloaded.List(); len(got) != 1 || !got[0].Installed || got[0].Enabled {
		t.Fatalf("persisted retry state=%+v", got)
	}
}

func TestRouterReloadDisablesNewModuleRequests(t *testing.T) {
	credential := []byte(strings.Repeat("router-test-", 4))
	router, err := NewRouter([]Module{pluginTestModule{"alpha", "1.0.0"}, pluginTestModule{"zeta", "1.0.0"}}, credential)
	if err != nil {
		t.Fatal(err)
	}
	if err := router.Reload([]string{"alpha"}); err != nil {
		t.Fatal(err)
	}
	request := func(path string) int {
		r := httptest.NewRequest(http.MethodGet, path, nil)
		r.Header.Set("Authorization", "Bearer "+string(credential))
		w := httptest.NewRecorder()
		router.ServeHTTP(w, r)
		return w.Code
	}
	if got := request("/extensions/v1/alpha/"); got != http.StatusOK {
		t.Fatalf("enabled status=%d", got)
	}
	if got := request("/extensions/v1/zeta/"); got != http.StatusNotFound {
		t.Fatalf("disabled status=%d", got)
	}
}
