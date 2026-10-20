// Package postgresurl validates the explicit Unix-socket PostgreSQL URL profile.
package postgresurl

import (
	"errors"
	"net/url"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"unicode"
	"unicode/utf8"
)

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
	for _, name := range []string{"PGHOST", "PGHOSTADDR", "PGPORT", "PGDATABASE", "PGUSER", "PGPASSWORD", "PGSERVICE", "PGSERVICEFILE", "PGREALM", "PGOPTIONS", "PGAPPNAME", "PGSSLMODE", "PGSSLCERT", "PGSSLKEY", "PGSSLROOTCERT", "PGSSLSNI", "PGREQUIRESSL", "PGSSLCRL", "PGREQUIREPEER", "PGKRBSRVNAME", "PGGSSLIB", "PGCONNECT_TIMEOUT", "PGCLIENTENCODING", "PGDATESTYLE", "PGTZ", "PGGEQO", "PGSYSCONFDIR", "PGLOCALEDIR", "PGPASSFILE"} {
		if _, present := os.LookupEnv(name); present {
			return true, errors.New("PostgreSQL connection environment is not supported with Unix sockets")
		}
	}
	return true, nil
}

func hasControl(value string) bool {
	return strings.IndexFunc(value, unicode.IsControl) >= 0
}
