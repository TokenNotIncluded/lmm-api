package main

import (
	"strings"
	"testing"
)

func TestBuildListenAddress(t *testing.T) {
	tests := []struct {
		name        string
		bindAddress string
		port        string
		want        string
		wantErr     string
	}{
		{
			name: "unset keeps the application listener private",
			port: "3000",
			want: "127.0.0.1:3000",
		},
		{
			name:        "IPv4 loopback",
			bindAddress: "127.0.0.1",
			port:        "3101",
			want:        "127.0.0.1:3101",
		},
		{
			name:        "IPv4 all interfaces",
			bindAddress: "0.0.0.0",
			port:        "3000",
			want:        "0.0.0.0:3000",
		},
		{
			name:        "IPv6 loopback",
			bindAddress: "::1",
			port:        "3101",
			want:        "[::1]:3101",
		},
		{
			name:        "IPv6 all interfaces",
			bindAddress: "::",
			port:        "3000",
			want:        "[::]:3000",
		},
		{
			name:        "surrounding Unicode whitespace is trimmed",
			bindAddress: "\u2003::1\u00a0",
			port:        "3101",
			want:        "[::1]:3101",
		},
		{
			name:        "whitespace-only address is rejected",
			bindAddress: " \t\u2003 ",
			port:        "3101",
			wantErr:     "must not contain only whitespace",
		},
		{
			name:        "IPv4 host and port is rejected",
			bindAddress: "127.0.0.1:3101",
			port:        "3101",
			wantErr:     "must be an IPv4 or IPv6 address without a port",
		},
		{
			name:        "IPv6 host and port is rejected",
			bindAddress: "[::1]:3101",
			port:        "3101",
			wantErr:     "must be an IPv4 or IPv6 address without a port",
		},
		{
			name:        "URL is rejected",
			bindAddress: "http://127.0.0.1",
			port:        "3101",
			wantErr:     "must be an IPv4 or IPv6 address without a port",
		},
		{
			name:        "hostname is rejected",
			bindAddress: "localhost",
			port:        "3101",
			wantErr:     "must be an IPv4 or IPv6 address without a port",
		},
		{
			name:        "DNS name is rejected",
			bindAddress: "api.lmm.best",
			port:        "3101",
			wantErr:     "must be an IPv4 or IPv6 address without a port",
		},
		{
			name:        "wildcard is rejected",
			bindAddress: "*",
			port:        "3101",
			wantErr:     "must be an IPv4 or IPv6 address without a port",
		},
		{
			name:        "bracketed IPv6 is rejected",
			bindAddress: "[::1]",
			port:        "3101",
			wantErr:     "must be an IPv4 or IPv6 address without a port",
		},
		{
			name:        "IPv6 zone is rejected",
			bindAddress: "fe80::1%lo",
			port:        "3101",
			wantErr:     "must be an IPv4 or IPv6 address without a port",
		},
		{
			name:        "path is rejected",
			bindAddress: "127.0.0.1/path",
			port:        "3101",
			wantErr:     "must be an IPv4 or IPv6 address without a port",
		},
		{
			name:        "backslash is rejected",
			bindAddress: `127.0.0.1\path`,
			port:        "3101",
			wantErr:     "must be an IPv4 or IPv6 address without a port",
		},
		{
			name:        "query is rejected",
			bindAddress: "127.0.0.1?interface=lo",
			port:        "3101",
			wantErr:     "must be an IPv4 or IPv6 address without a port",
		},
		{
			name:        "fragment is rejected",
			bindAddress: "127.0.0.1#loopback",
			port:        "3101",
			wantErr:     "must be an IPv4 or IPv6 address without a port",
		},
		{
			name:        "userinfo is rejected",
			bindAddress: "user@127.0.0.1",
			port:        "3101",
			wantErr:     "must be an IPv4 or IPv6 address without a port",
		},
		{
			name:        "malformed label is rejected",
			bindAddress: "-invalid-host",
			port:        "3101",
			wantErr:     "must be an IPv4 or IPv6 address without a port",
		},
		{
			name:        "embedded ASCII whitespace is rejected",
			bindAddress: "127.0. 0.1",
			port:        "3101",
			wantErr:     "must be an IPv4 or IPv6 address without a port",
		},
		{
			name:        "embedded Unicode whitespace is rejected",
			bindAddress: "127.0.\u20030.1",
			port:        "3101",
			wantErr:     "must be an IPv4 or IPv6 address without a port",
		},
		{
			name:        "control character is rejected",
			bindAddress: "127.0.0.1\x00",
			port:        "3101",
			wantErr:     "must be an IPv4 or IPv6 address without a port",
		},
	}

	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			got, err := buildListenAddress(test.bindAddress, test.port)
			if test.wantErr != "" {
				if err == nil {
					t.Fatalf("buildListenAddress() error = nil, want error containing %q", test.wantErr)
				}
				if !strings.Contains(err.Error(), test.wantErr) {
					t.Fatalf("buildListenAddress() error = %q, want error containing %q", err, test.wantErr)
				}
				return
			}
			if err != nil {
				t.Fatalf("buildListenAddress() unexpected error: %v", err)
			}
			if got != test.want {
				t.Fatalf("buildListenAddress() = %q, want %q", got, test.want)
			}
		})
	}
}

func TestResolvePort(t *testing.T) {
	tests := []struct {
		name          string
		primary       string
		compatibility string
		fallback      int
		want          string
	}{
		{name: "PORT wins", primary: "3100", compatibility: "3200", fallback: 3000, want: "3100"},
		{name: "legacy compose name is supported", compatibility: "3200", fallback: 3000, want: "3200"},
		{name: "blank values use flag default", primary: " \t", compatibility: "", fallback: 3000, want: "3000"},
	}

	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			if got := resolvePort(test.primary, test.compatibility, test.fallback); got != test.want {
				t.Fatalf("resolvePort() = %q, want %q", got, test.want)
			}
		})
	}
}

func TestPprofListenAddress(t *testing.T) {
	tests := []struct {
		name        string
		bindAddress string
		port        string
		want        string
		wantErr     bool
	}{
		{name: "secure loopback default", want: "127.0.0.1:8005"},
		{name: "explicit IPv6 and port", bindAddress: "::1", port: "18005", want: "[::1]:18005"},
		{name: "invalid port", bindAddress: "127.0.0.1", port: "65536", wantErr: true},
		{name: "hostname is rejected", bindAddress: "localhost", port: "18005", wantErr: true},
	}

	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			got, err := pprofListenAddress(test.bindAddress, test.port)
			if test.wantErr {
				if err == nil {
					t.Fatal("pprofListenAddress() error = nil, want error")
				}
				return
			}
			if err != nil {
				t.Fatalf("pprofListenAddress() unexpected error: %v", err)
			}
			if got != test.want {
				t.Fatalf("pprofListenAddress() = %q, want %q", got, test.want)
			}
		})
	}
}

func TestLocalAcceptancePolicy(t *testing.T) {
	tests := []struct {
		name           string
		flag           string
		configuredBind string
		listenAddress  string
		want           bool
		wantErr        bool
	}{
		{name: "missing flag preserves production", configuredBind: "0.0.0.0", listenAddress: "0.0.0.0:3000"},
		{name: "false flag preserves production", flag: "false", configuredBind: "0.0.0.0", listenAddress: "0.0.0.0:3000"},
		{name: "non-exact flag remains disabled", flag: "TRUE", configuredBind: "127.0.0.1", listenAddress: "127.0.0.1:3101"},
		{name: "IPv4 loopback", flag: "true", configuredBind: "127.0.0.1", listenAddress: "127.0.0.1:3101", want: true},
		{name: "IPv6 loopback", flag: "true", configuredBind: "::1", listenAddress: "[::1]:3101", want: true},
		{name: "missing configured bind", flag: "true", listenAddress: "127.0.0.1:3101", wantErr: true},
		{name: "hostname configured bind", flag: "true", configuredBind: "localhost", listenAddress: "127.0.0.1:3101", wantErr: true},
		{name: "URL configured bind", flag: "true", configuredBind: "http://127.0.0.1", listenAddress: "127.0.0.1:3101", wantErr: true},
		{name: "IPv4 wildcard", flag: "true", configuredBind: "0.0.0.0", listenAddress: "0.0.0.0:3101", wantErr: true},
		{name: "IPv6 wildcard", flag: "true", configuredBind: "::", listenAddress: "[::]:3101", wantErr: true},
		{name: "other loopback address rejected", flag: "true", configuredBind: "127.0.0.2", listenAddress: "127.0.0.2:3101", wantErr: true},
		{name: "mapped IPv6 configured bind rejected", flag: "true", configuredBind: "::ffff:127.0.0.1", listenAddress: "[::ffff:127.0.0.1]:3101", wantErr: true},
		{name: "mapped IPv6 final host rejected", flag: "true", configuredBind: "127.0.0.1", listenAddress: "[::ffff:127.0.0.1]:3101", wantErr: true},
		{name: "scoped IPv6 configured bind rejected", flag: "true", configuredBind: "::1%lo", listenAddress: "[::1%lo]:3101", wantErr: true},
		{name: "configured and final host disagree", flag: "true", configuredBind: "127.0.0.1", listenAddress: "[::1]:3101", want: true},
		{name: "malformed final address", flag: "true", configuredBind: "127.0.0.1", listenAddress: "127.0.0.1", wantErr: true},
		{name: "hostname final address", flag: "true", configuredBind: "127.0.0.1", listenAddress: "localhost:3101", wantErr: true},
		{name: "ambiguous final port", flag: "true", configuredBind: "127.0.0.1", listenAddress: "127.0.0.1:http", wantErr: true},
		{name: "out of range final port", flag: "true", configuredBind: "127.0.0.1", listenAddress: "127.0.0.1:65536", wantErr: true},
	}

	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			got, err := localAcceptancePolicy(test.flag, test.configuredBind, test.listenAddress)
			if test.wantErr {
				if err == nil {
					t.Fatal("localAcceptancePolicy() error = nil, want an error")
				}
				return
			}
			if err != nil {
				t.Fatalf("localAcceptancePolicy() unexpected error: %v", err)
			}
			if got != test.want {
				t.Fatalf("localAcceptancePolicy() = %t, want %t", got, test.want)
			}
		})
	}
}

func TestRegionalBindPolicy(t *testing.T) {
	tests := []struct {
		name           string
		enabled        bool
		configuredBind string
		listenAddress  string
		wantErr        bool
	}{
		{name: "disabled policy allows explicit public bind", configuredBind: "0.0.0.0", listenAddress: "0.0.0.0:3000"},
		{name: "default bind remains private", enabled: true, listenAddress: "127.0.0.1:3000"},
		{name: "loopback IPv6 remains private", enabled: true, configuredBind: "::1", listenAddress: "[::1]:3000"},
		{name: "public configured bind is rejected", enabled: true, configuredBind: "0.0.0.0", listenAddress: "0.0.0.0:3000", wantErr: true},
		{name: "public final bind is rejected", enabled: true, configuredBind: "127.0.0.1", listenAddress: "0.0.0.2:3000", wantErr: true},
		{name: "malformed final address is rejected", enabled: true, configuredBind: "127.0.0.1", listenAddress: "127.0.0.1", wantErr: true},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			err := regionalBindPolicy(test.enabled, test.configuredBind, test.listenAddress)
			if test.wantErr && err == nil {
				t.Fatal("regionalBindPolicy() error = nil, want error")
			}
			if !test.wantErr && err != nil {
				t.Fatalf("regionalBindPolicy() unexpected error: %v", err)
			}
		})
	}
}
