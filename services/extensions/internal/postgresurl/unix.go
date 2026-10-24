// Package postgresurl validates the explicit Unix-socket PostgreSQL URL profile.
package postgresurl

import (
	"errors"
	"net/netip"
	"net/url"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"unicode"
	"unicode/utf8"
)

var connectionEnvironment = []string{"PGHOST", "PGHOSTADDR", "PGPORT", "PGDATABASE", "PGUSER", "PGPASSWORD", "PGSERVICE", "PGSERVICEFILE", "PGREALM", "PGOPTIONS", "PGAPPNAME", "PGSSLMODE", "PGSSLCERT", "PGSSLKEY", "PGSSLROOTCERT", "PGSSLSNI", "PGREQUIRESSL", "PGSSLCRL", "PGREQUIREPEER", "PGKRBSRVNAME", "PGGSSLIB", "PGCONNECT_TIMEOUT", "PGCLIENTENCODING", "PGDATESTYLE", "PGTZ", "PGGEQO", "PGSYSCONFDIR", "PGLOCALEDIR", "PGPASSFILE"}

// TCP validates the explicit transport policy for extension PostgreSQL URLs.
// Numeric loopback is allowed to use plaintext for same-host deployments; every
// other TCP endpoint must authenticate the server with verify-full TLS.
func TCP(u *url.URL) (bool, error) {
	if u == nil || u.Hostname() == "" {
		return false, nil
	}
	if u.User == nil || u.User.Username() == "" || hasControl(u.User.Username()) {
		return true, errors.New("TCP PostgreSQL URLs require an explicit user")
	}
	password, present := u.User.Password()
	if !present || password == "" || hasControl(password) {
		return true, errors.New("TCP PostgreSQL URLs require an explicit password")
	}
	q, err := url.ParseQuery(u.RawQuery)
	if err != nil {
		return true, errors.New("invalid TCP PostgreSQL URL")
	}
	for _, name := range connectionEnvironment {
		if _, present := os.LookupEnv(name); present {
			return true, errors.New("PostgreSQL connection environment is not supported with TCP URLs")
		}
	}
	addr, err := netip.ParseAddr(u.Hostname())
	if err == nil && addr.IsLoopback() && q.Get("sslmode") == "disable" {
		return true, nil
	}
	if q.Get("sslmode") != "verify-full" {
		return true, errors.New("non-loopback TCP PostgreSQL URLs require sslmode=verify-full")
	}
	if rootCert := q.Get("sslrootcert"); rootCert == "" || !filepath.IsAbs(rootCert) || filepath.Clean(rootCert) != rootCert || strings.ContainsAny(rootCert, "\r\n\x00") {
		return true, errors.New("TCP PostgreSQL sslrootcert must be an absolute path")
	}
	return true, nil
}

// Unix returns false for TCP URLs, whose existing caller policy remains in force.
// Unix credentials and endpoints must be explicit: lib/pq otherwise inherits
// connection options from the process environment and may consult .pgpass.
func Unix(u *url.URL) (bool, error) {
	if u == nil {
		return false, errors.New("invalid PostgreSQL URL")
	}
	if u.Hostname() != "" {
		return false, nil
	}
	reject := func() (bool, error) { return true, errors.New("invalid explicit Unix PostgreSQL URL") }
	if u.Host != "" || (u.Scheme != "postgres" && u.Scheme != "postgresql") || u.User == nil || u.User.Username() == "" || hasControl(u.User.Username()) {
		return reject()
	}
	password, present := u.User.Password()
	if !present || password == "" || hasControl(password) {
		return reject()
	}
	q, err := url.ParseQuery(u.RawQuery)
	if err != nil {
		return reject()
	}
	for key, values := range q {
		if len(values) != 1 || (key != "host" && key != "port" && key != "sslmode") {
			return reject()
		}
	}
	host := q.Get("host")
	if !utf8.ValidString(host) || !filepath.IsAbs(host) || filepath.Clean(host) != host || hasControl(host) || strings.ContainsAny(host, ",\\") || q.Get("sslmode") != "disable" {
		return reject()
	}
	if ports, ok := q["port"]; ok {
		port, err := strconv.Atoi(ports[0])
		if err != nil || port < 1024 || port > 65535 || strconv.Itoa(port) != ports[0] {
			return reject()
		}
	}
	// This is the complete environment list consumed by pinned lib/pq v1.10.9
	// parseEnviron plus PGPASSFILE. Presence is rejected even for empty values:
	// lib/pq panics on some unsupported variables before parsing the explicit URL.
	for _, name := range connectionEnvironment {
		if _, present := os.LookupEnv(name); present {
			return true, errors.New("PostgreSQL connection environment is not supported with Unix sockets")
		}
	}
	return true, nil
}

func hasControl(value string) bool {
	return strings.IndexFunc(value, unicode.IsControl) >= 0
}
