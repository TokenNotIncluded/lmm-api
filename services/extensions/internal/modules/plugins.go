package modules

// This file implements the extension plugin lifecycle. Plugins are trusted,
// compiled Go modules registered by the host binary. The lifecycle is dynamic
// at the route boundary, while code installation remains a reviewed release
// operation; arbitrary shared libraries are never loaded into the process.

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"regexp"
	"sort"
	"strings"
	"sync"
)

var (
	ErrPluginNotFound       = errors.New("plugin is not registered")
	ErrPluginNotInstalled   = errors.New("plugin is not installed")
	ErrPluginAlreadyEnabled = errors.New("plugin is already enabled")
	ErrPluginAlreadyOff     = errors.New("plugin is already disabled")
	ErrPluginManifest       = errors.New("invalid plugin manifest")
	ErrPluginState          = errors.New("invalid plugin state")
)

const (
	PluginManifestVersion = 1
	PluginAPIVersion      = 1
	maxPluginStateBytes   = 64 << 10
)

var pluginVersionPattern = regexp.MustCompile(`^[0-9A-Za-z][0-9A-Za-z._-]{0,63}$`)

// PluginInfo is the public, non-secret lifecycle view returned to an
// administrator. RegistrationSHA256 binds the request to a compiled plugin
// registration, not to untrusted source supplied over HTTP.
type PluginInfo struct {
	Name               string `json:"name"`
	Version            string `json:"version"`
	APIVersion         int    `json:"api_version"`
	RegistrationSHA256 string `json:"registration_sha256"`
	Available          bool   `json:"available"`
	Installed          bool   `json:"installed"`
	Enabled            bool   `json:"enabled"`
}

type PluginInstallRequest struct {
	Name               string `json:"name"`
	Version            string `json:"version"`
	APIVersion         int    `json:"api_version"`
	RegistrationSHA256 string `json:"registration_sha256"`
}

type pluginStateFile struct {
	FormatVersion int           `json:"format_version"`
	Plugins       []pluginState `json:"plugins"`
}

type pluginState struct {
	Name               string `json:"name"`
	Version            string `json:"version"`
	APIVersion         int    `json:"api_version"`
	RegistrationSHA256 string `json:"registration_sha256"`
	Installed          bool   `json:"installed"`
	Enabled            bool   `json:"enabled"`
}

type pluginEntry struct {
	info   PluginInfo
	module Module
}

// ModuleReloader is deliberately narrow so lifecycle tests can use a fake and
// the manager cannot mutate storage or bypass Host admission.
type ModuleReloader interface {
	ReloadModules([]string) error
}

// PluginManager owns the enabled snapshot and persists only lifecycle state.
// It never accepts executable bytes, imports a Go plugin, or grants a module
// core authority; those properties are fixed by the compiled registration.
type PluginManager struct {
	mu        sync.RWMutex
	reloader  ModuleReloader
	stateFile string
	entries   map[string]pluginEntry
	installed map[string]bool
	enabled   map[string]bool
}

func RegistrationSHA256(name, version string) string {
	h := sha256.Sum256([]byte("lmm-extension-registration-v1\x00" + name + "\x00" + version))
	return hex.EncodeToString(h[:])
}

func NewPluginManager(reloader ModuleReloader, registered []Module, stateFile string) (*PluginManager, error) {
	if reloader == nil {
		return nil, errors.New("plugin manager requires a module reloader")
	}
	if stateFile != "" {
		if !filepath.IsAbs(stateFile) || filepath.Clean(stateFile) != stateFile || strings.ContainsAny(stateFile, "\x00\r\n") {
			return nil, errors.New("plugin state file must be an absolute clean path")
		}
	}
	m := &PluginManager{reloader: reloader, stateFile: stateFile, entries: make(map[string]pluginEntry, len(registered)), installed: make(map[string]bool, len(registered)), enabled: make(map[string]bool, len(registered))}
	for _, module := range registered {
		if isNil(module) || !validName.MatchString(module.Name()) || m.entries[module.Name()].module != nil || isNil(module.Handler()) {
			return nil, ErrPluginManifest
		}
		version := "1.0.0"
		if versioned, ok := module.(interface{ Version() string }); ok && versioned.Version() != "" {
			version = versioned.Version()
		}
		if !pluginVersionPattern.MatchString(version) {
			return nil, ErrPluginManifest
		}
		m.entries[module.Name()] = pluginEntry{module: module, info: PluginInfo{
			Name: module.Name(), Version: version, APIVersion: PluginAPIVersion,
			RegistrationSHA256: RegistrationSHA256(module.Name(), version), Available: true,
		}}
	}
	loaded, exists, err := readPluginState(stateFile)
	if err != nil {
		return nil, err
	}
	if !exists {
		for name := range m.entries {
			m.installed[name], m.enabled[name] = true, true
		}
		if err := m.persistLocked(); err != nil {
			return nil, err
		}
	} else {
		for name := range m.entries {
			m.installed[name], m.enabled[name] = false, false
		}
		for _, state := range loaded {
			entry, ok := m.entries[state.Name]
			if !ok || state.Version != entry.info.Version || state.APIVersion != entry.info.APIVersion || state.RegistrationSHA256 != entry.info.RegistrationSHA256 || (state.Enabled && !state.Installed) {
				return nil, ErrPluginState
			}
			m.installed[state.Name], m.enabled[state.Name] = state.Installed, state.Enabled
		}
	}
	if err := m.reloadLocked(); err != nil {
		return nil, err
	}
	return m, nil
}

func (m *PluginManager) List() []PluginInfo {
	m.mu.RLock()
	defer m.mu.RUnlock()
	result := make([]PluginInfo, 0, len(m.entries))
	for name, entry := range m.entries {
		info := entry.info
		info.Installed, info.Enabled = m.installed[name], m.enabled[name]
		result = append(result, info)
	}
	sort.Slice(result, func(i, j int) bool { return result[i].Name < result[j].Name })
	return result
}

func (m *PluginManager) IsEnabled(name string) bool {
	m.mu.RLock()
	defer m.mu.RUnlock()
	return m.enabled[name]
}

func (m *PluginManager) Install(request PluginInstallRequest) (PluginInfo, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	entry, ok := m.entries[request.Name]
	if !ok {
		return PluginInfo{}, ErrPluginNotFound
	}
	if request.APIVersion != entry.info.APIVersion || request.Version != entry.info.Version || request.RegistrationSHA256 != entry.info.RegistrationSHA256 {
		return PluginInfo{}, ErrPluginManifest
	}
	m.installed[request.Name] = true
	if err := m.persistLocked(); err != nil {
		return PluginInfo{}, err
	}
	return m.infoLocked(request.Name), nil
}

func (m *PluginManager) Enable(name string) (PluginInfo, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	entry, ok := m.entries[name]
	if !ok {
		return PluginInfo{}, ErrPluginNotFound
	}
	if !m.installed[name] {
		return PluginInfo{}, ErrPluginNotInstalled
	}
	if m.enabled[name] {
		return m.infoLocked(name), ErrPluginAlreadyEnabled
	}
	m.enabled[name] = true
	if err := m.reloadLocked(); err != nil {
		m.enabled[name] = false
		return PluginInfo{}, err
	}
	if err := m.persistLocked(); err != nil {
		m.enabled[name] = false
		_ = m.reloadLocked()
		return PluginInfo{}, err
	}
	return m.infoLocked(entry.info.Name), nil
}

func (m *PluginManager) Disable(name string) (PluginInfo, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	if _, ok := m.entries[name]; !ok {
		return PluginInfo{}, ErrPluginNotFound
	}
	if !m.enabled[name] {
		return m.infoLocked(name), ErrPluginAlreadyOff
	}
	m.enabled[name] = false
	if err := m.reloadLocked(); err != nil {
		m.enabled[name] = true
		return PluginInfo{}, err
	}
	if err := m.persistLocked(); err != nil {
		m.enabled[name] = true
		_ = m.reloadLocked()
		return PluginInfo{}, err
	}
	return m.infoLocked(name), nil
}

func (m *PluginManager) Uninstall(name string) (PluginInfo, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	if _, ok := m.entries[name]; !ok {
		return PluginInfo{}, ErrPluginNotFound
	}
	if m.enabled[name] {
		return PluginInfo{}, errors.New("disable plugin before uninstall")
	}
	m.installed[name] = false
	if err := m.persistLocked(); err != nil {
		m.installed[name] = true
		return PluginInfo{}, err
	}
	return m.infoLocked(name), nil
}

func (m *PluginManager) Reload() error {
	m.mu.Lock()
	defer m.mu.Unlock()
	return m.reloadLocked()
}

func (m *PluginManager) reloadLocked() error {
	enabled := make([]string, 0, len(m.enabled))
	for name, on := range m.enabled {
		if on {
			enabled = append(enabled, name)
		}
	}
	sort.Strings(enabled)
	return m.reloader.ReloadModules(enabled)
}

func (m *PluginManager) infoLocked(name string) PluginInfo {
	info := m.entries[name].info
	info.Installed, info.Enabled = m.installed[name], m.enabled[name]
	return info
}

func (m *PluginManager) persistLocked() error {
	if m.stateFile == "" {
		return nil
	}
	directory := filepath.Dir(m.stateFile)
	if err := os.MkdirAll(directory, 0700); err != nil {
		return fmt.Errorf("create plugin state directory: %w", err)
	}
	if info, err := os.Stat(directory); err != nil || !info.IsDir() || info.Mode().Perm()&0077 != 0 {
		return ErrPluginState
	}
	state := pluginStateFile{FormatVersion: PluginManifestVersion}
	for name, entry := range m.entries {
		state.Plugins = append(state.Plugins, pluginState{Name: name, Version: entry.info.Version,
			APIVersion: entry.info.APIVersion, RegistrationSHA256: entry.info.RegistrationSHA256,
			Installed: m.installed[name], Enabled: m.enabled[name]})
	}
	sort.Slice(state.Plugins, func(i, j int) bool { return state.Plugins[i].Name < state.Plugins[j].Name })
	raw, err := json.Marshal(state)
	if err != nil {
		return err
	}
	tmp, err := os.CreateTemp(directory, ".plugins-*.tmp")
	if err != nil {
		return err
	}
	tmpName := tmp.Name()
	defer os.Remove(tmpName)
	if err = tmp.Chmod(0600); err == nil {
		_, err = tmp.Write(append(raw, '\n'))
	}
	if err == nil {
		err = tmp.Sync()
	}
	if closeErr := tmp.Close(); err == nil {
		err = closeErr
	}
	if err != nil {
		return err
	}
	if err := os.Rename(tmpName, m.stateFile); err != nil {
		return err
	}
	return nil
}

func readPluginState(path string) ([]pluginState, bool, error) {
	if path == "" {
		return nil, false, nil
	}
	info, err := os.Lstat(path)
	if errors.Is(err, os.ErrNotExist) {
		return nil, false, nil
	}
	if err != nil || info.Mode()&os.ModeSymlink != 0 || !info.Mode().IsRegular() || info.Mode().Perm()&0077 != 0 || info.Size() > maxPluginStateBytes {
		return nil, false, ErrPluginState
	}
	f, err := os.Open(path)
	if err != nil {
		return nil, false, ErrPluginState
	}
	defer f.Close()
	raw, err := io.ReadAll(io.LimitReader(f, maxPluginStateBytes+1))
	if err != nil || len(raw) > maxPluginStateBytes {
		return nil, false, ErrPluginState
	}
	decoder := json.NewDecoder(strings.NewReader(string(raw)))
	decoder.DisallowUnknownFields()
	var state pluginStateFile
	if err := decoder.Decode(&state); err != nil || state.FormatVersion != PluginManifestVersion {
		return nil, false, ErrPluginState
	}
	var extra any
	if err := decoder.Decode(&extra); !errors.Is(err, io.EOF) {
		return nil, false, ErrPluginState
	}
	seen := map[string]bool{}
	for _, item := range state.Plugins {
		if !validName.MatchString(item.Name) || !pluginVersionPattern.MatchString(item.Version) || item.APIVersion != PluginAPIVersion || !validDigest(item.RegistrationSHA256) || seen[item.Name] {
			return nil, false, ErrPluginState
		}
		seen[item.Name] = true
	}
	return state.Plugins, true, nil
}

func validDigest(value string) bool {
	if len(value) != sha256.Size*2 {
		return false
	}
	_, err := hex.DecodeString(value)
	return err == nil
}
