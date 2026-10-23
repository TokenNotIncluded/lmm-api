// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package composition

import (
	"context"
	"database/sql"
	"encoding/json"
	"os"
	"regexp"
	"strings"
	"testing"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/coreclient"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/freshinstall"
	"github.com/lib/pq"
)

func TestPostgresToolMarketBuiltinBundleGuard(t *testing.T) {
	path := os.Getenv("LMM_TOOLMARKET_BUNDLE_GUARD_MANIFEST")
	if path == "" {
		t.Skip("requires a newly owned isolated PostgreSQL 18 fixture")
	}
	var f struct {
		OwnerURLFile, RuntimeURLFile, AdminURLFile, KeyFile string
		Database, OwnerRole, RuntimeRole, ProbeDatabase     string
	}
	raw, err := coreclient.ReadPrivateFile(path, 8192)
	if err != nil || json.Unmarshal(raw, &f) != nil || !regexp.MustCompile(`^lmm_toolmarket_[a-f0-9]{12}_guard$`).MatchString(f.Database) || !regexp.MustCompile(`^a09_[a-f0-9]{12}_a08_[or]$`).MatchString(f.OwnerRole) || !regexp.MustCompile(`^a09_[a-f0-9]{12}_a08_r$`).MatchString(f.RuntimeRole) || !regexp.MustCompile(`^lmm_store_[a-f0-9]{12}_probe$`).MatchString(f.ProbeDatabase) {
		t.Fatal("invalid private fresh fixture")
	}
	ctx := context.Background()
	if _, err := freshinstall.Install(ctx, freshinstall.Config{OwnerURLFile: f.OwnerURLFile, ExpectedDatabase: f.Database, OwnerRole: f.OwnerRole, RuntimeRole: f.RuntimeRole, Component: "toolmarket-builtins", EncryptionKeyFile: f.KeyFile}); err != nil {
		t.Fatal("formal fresh builtin installation failed")
	}
	dsn, err := coreclient.ReadPrivateFile(f.AdminURLFile, 8192)
	if err != nil {
		t.Fatal("private fixture administrator unavailable")
	}
	admin, err := sql.Open("postgres", strings.TrimSpace(string(dsn)))
	if err != nil {
		t.Fatal("private fixture administrator unavailable")
	}
	defer admin.Close()
	cfg := StorageConfig{Component: "toolmarket", URLFile: f.RuntimeURLFile, Database: f.Database}
	exec := func(query string) {
		t.Helper()
		if _, err := admin.ExecContext(ctx, query); err != nil {
			t.Fatal("owned fixture mutation failed")
		}
	}
	accept := func() {
		t.Helper()
		db, err := openStorageBundle(ctx, cfg, toolMarketBuiltinsBundle)
		if err != nil {
			t.Fatal("explicit installed bundle rejected", err)
		}
		db.Close()
	}
	reject := func() {
		t.Helper()
		db, err := openStorageBundle(ctx, cfg, toolMarketBuiltinsBundle)
		if err == nil {
			db.Close()
			t.Fatal("unisolated bundle accepted")
		}
	}
	accept()
	t.Run("plain component rejects bundled namespaces", func(t *testing.T) {
		if db, err := openStorage(ctx, cfg); err == nil {
			db.Close()
			t.Fatal("default storage implicitly enabled builtins")
		}
	})
	for _, namespace := range []string{"lmm_assistant", "extension_unregistered", "unregistered", "core_probe"} {
		t.Run("foreign schema "+namespace, func(t *testing.T) {
			exec("CREATE SCHEMA " + pq.QuoteIdentifier(namespace))
			defer exec("DROP SCHEMA " + pq.QuoteIdentifier(namespace))
			reject()
		})
	}
	role := pq.QuoteIdentifier(f.RuntimeRole)
	t.Run("missing minimal table grant", func(t *testing.T) {
		exec("REVOKE SELECT ON lmm_bounties.contract FROM " + role)
		defer exec("GRANT SELECT ON lmm_bounties.contract TO " + role)
		reject()
	})
	policy, _ := storagePolicyFor("toolmarket", toolMarketBuiltinsBundle)
	for _, table := range policy.forbidden {
		for _, privilege := range table.privileges {
			t.Run("table privilege "+table.name+" "+privilege, func(t *testing.T) {
				exec("GRANT " + privilege + " ON " + table.name + " TO " + role)
				defer exec("REVOKE " + privilege + " ON " + table.name + " FROM " + role)
				reject()
			})
		}
	}
	for _, table := range policy.forbiddenColumns {
		var column string
		if err := admin.QueryRowContext(ctx, `SELECT attname FROM pg_attribute WHERE attrelid=to_regclass($1) AND attnum>0 AND NOT attisdropped ORDER BY attnum LIMIT 1`, table.name).Scan(&column); err != nil {
			t.Fatal("fixture column unavailable")
		}
		for _, privilege := range table.privileges {
			t.Run("column privilege "+table.name+" "+privilege, func(t *testing.T) {
				spec := privilege + "(" + pq.QuoteIdentifier(column) + ") ON " + table.name
				exec("GRANT " + spec + " TO " + role)
				defer exec("REVOKE " + spec + " FROM " + role)
				reject()
			})
		}
	}
	t.Run("cross database connect", func(t *testing.T) {
		database := pq.QuoteIdentifier(f.ProbeDatabase)
		exec("GRANT CONNECT ON DATABASE " + database + " TO " + role)
		defer exec("REVOKE CONNECT ON DATABASE " + database + " FROM " + role)
		reject()
	})
	t.Run("hidden role path", func(t *testing.T) {
		hidden := pq.QuoteIdentifier(f.RuntimeRole + "_probe")
		exec("CREATE ROLE " + hidden + " NOLOGIN")
		defer exec("DROP ROLE " + hidden)
		exec("GRANT " + hidden + " TO " + role + " WITH INHERIT FALSE, SET TRUE")
		defer exec("REVOKE " + hidden + " FROM " + role)
		reject()
	})
	accept()
}
