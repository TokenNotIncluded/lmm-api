// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package freshinstall

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"net/url"
	"os"
	"regexp"
	"strings"
	"testing"
	"time"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/bounties"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/payments"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/toolmarket"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/wallet"
	"github.com/lib/pq"
)

type installFixture struct {
	OwnerURLFile     string `json:"owner_url_file"`
	RuntimeURLFile   string `json:"runtime_url_file"`
	AdminURLFile     string `json:"admin_url_file"`
	CoreURLFile      string `json:"core_url_file"`
	KeyFile          string `json:"key_file"`
	ExpectedDatabase string `json:"expected_database"`
	OwnerRole        string `json:"owner_role"`
	InstallerRole    string `json:"installer_role"`
	RuntimeRole      string `json:"runtime_role"`
	Component        string `json:"component"`
	CoreDatabase     string `json:"core_database"`
}

func fixtureCases(t *testing.T) map[string]installFixture {
	t.Helper()
	path := os.Getenv("LMM_FRESH_INSTALL_TEST_MANIFEST")
	if path == "" {
		t.Skip("requires a newly provisioned private installer-only PostgreSQL fixture")
	}
	raw, e := os.ReadFile(path)
	if e != nil {
		t.Fatal("read private fixture manifest")
	}
	var cases map[string]installFixture
	if json.Unmarshal(raw, &cases) != nil || len(cases) != 10 {
		t.Fatal("invalid fixture cases")
	}
	for _, f := range cases {
		if !regexp.MustCompile(`^lmm_(assistant|toolmarket|payments)_[a-f0-9]{12}_[a-z]+$`).MatchString(f.ExpectedDatabase) || !strings.HasPrefix(f.OwnerRole, "a09_") {
			t.Fatal("fresh synthetic fixture required")
		}
	}
	return cases
}
func (f installFixture) config() Config {
	key := ""
	if strings.HasPrefix(f.Component, "toolmarket") {
		key = f.KeyFile
	}
	return Config{OwnerURLFile: f.OwnerURLFile, ExpectedDatabase: f.ExpectedDatabase, OwnerRole: f.OwnerRole, RuntimeRole: f.RuntimeRole, Component: f.Component, EncryptionKeyFile: key}
}
func privateDB(t *testing.T, path string) *sql.DB {
	t.Helper()
	raw, e := os.ReadFile(path)
	if e != nil {
		t.Fatal("read private local connection file")
	}
	u, e := url.Parse(strings.TrimSpace(string(raw)))
	if e != nil || u.Scheme != "postgres" || u.Hostname() != "127.0.0.1" {
		t.Fatal("private numeric-loopback fixture required")
	}
	db, e := sql.Open("postgres", u.String())
	if e != nil {
		t.Fatal("open private fixture")
	}
	t.Cleanup(func() { db.Close() })
	return db
}
func scalar(t *testing.T, db *sql.DB, query string, args ...any) int {
	t.Helper()
	var n int
	if e := db.QueryRow(query, args...).Scan(&n); e != nil {
		t.Fatal("private fixture scalar failed")
	}
	return n
}
func execPrivate(t *testing.T, db *sql.DB, query string, args ...any) {
	t.Helper()
	if _, e := db.Exec(query, args...); e != nil {
		t.Fatal("private fixture SQL failed")
	}
}
func denied(t *testing.T, e error) {
	t.Helper()
	var pg *pq.Error
	if !errors.As(e, &pg) || pg.Code != "42501" {
		t.Fatal("expected actual PostgreSQL permission denial SQLSTATE42501")
	}
}
func noApplicationObjects(t *testing.T, db *sql.DB) {
	t.Helper()
	if scalar(t, db, `SELECT count(*) FROM pg_namespace WHERE nspname NOT IN ('public','information_schema') AND nspname NOT LIKE 'pg\_%' ESCAPE '\'`) != 0 || scalar(t, db, `SELECT count(*) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public'`) != 0 {
		t.Fatal("failed install left application objects")
	}
}
func mustInstall(t *testing.T, f installFixture) Receipt {
	t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), 40*time.Second)
	defer cancel()
	receipt, e := Install(ctx, f.config())
	if e != nil {
		t.Fatal("fresh install rejected valid fixture")
	}
	if receipt.Component != f.Component || receipt.Database != f.ExpectedDatabase || !regexp.MustCompile(`^[a-f0-9]{64}$`).MatchString(receipt.SchemaSHA256) {
		t.Fatal("invalid install receipt")
	}
	return receipt
}

func TestFreshInstallRealOwnerRoleRuntimeIsolationAndEncryptedBuiltinStores(t *testing.T) {
	f := fixtureCases(t)["builtins"]
	ctx := context.Background()
	admin := privateDB(t, f.AdminURLFile)
	runtimeDB := privateDB(t, f.RuntimeURLFile)
	ownerDB := privateDB(t, f.OwnerURLFile)
	var session, current, dbowner string
	if ownerDB.QueryRow(`SELECT session_user,current_user,pg_get_userbyid(datdba) FROM pg_database WHERE datname=current_database()`).Scan(&session, &current, &dbowner) != nil || session != f.InstallerRole || current != f.InstallerRole || dbowner != f.OwnerRole {
		t.Fatal("not a real LOGIN installer and separate NOLOGIN datdba")
	}
	if scalar(t, admin, `SELECT count(*) FROM pg_roles WHERE rolname=$1 AND NOT rolcanlogin AND NOT rolsuper AND NOT rolcreatedb AND NOT rolcreaterole AND NOT rolreplication AND NOT rolbypassrls`, f.OwnerRole) != 1 {
		t.Fatal("owner is not constrained NOLOGIN role")
	}
	cfg := f.config()
	cfg.OwnerURLFile = f.RuntimeURLFile
	if _, e := Install(ctx, cfg); e == nil {
		t.Fatal("runtime installed schema")
	}
	noApplicationObjects(t, admin)
	cfg = f.config()
	cfg.EncryptionKeyFile = ""
	if _, e := Install(ctx, cfg); e == nil {
		t.Fatal("missing encryption key accepted")
	}
	noApplicationObjects(t, admin)
	cfg = f.config()
	cfg.EncryptionKeyFile = f.OwnerURLFile
	if _, e := Install(ctx, cfg); e == nil {
		t.Fatal("reused credential file accepted as encryption key")
	}
	noApplicationObjects(t, admin)
	mustInstall(t, f)
	if scalar(t, admin, `SELECT count(*) FROM pg_namespace WHERE nspname IN ('lmm_toolmarket','extension_wallet','lmm_bounties') AND pg_get_userbyid(nspowner)=$1`, f.OwnerRole) != 3 {
		t.Fatal("schemas do not belong to immutable database owner")
	}
	var allowed bool
	checks := []string{
		`has_table_privilege(current_user,'lmm_toolmarket.snapshot','SELECT') AND has_table_privilege(current_user,'lmm_toolmarket.snapshot','UPDATE') AND NOT has_table_privilege(current_user,'lmm_toolmarket.snapshot','INSERT,DELETE,TRUNCATE')`,
		`has_table_privilege(current_user,'extension_wallet.transfer_links','SELECT') AND has_table_privilege(current_user,'extension_wallet.transfer_links','INSERT') AND NOT has_table_privilege(current_user,'extension_wallet.transfer_links','UPDATE,DELETE,TRUNCATE')`,
		`has_table_privilege(current_user,'lmm_bounties.contract','SELECT') AND NOT has_table_privilege(current_user,'lmm_bounties.contract','INSERT,UPDATE,DELETE,TRUNCATE')`,
		`has_table_privilege(current_user,'lmm_bounties.records','SELECT') AND has_table_privilege(current_user,'lmm_bounties.records','INSERT') AND has_table_privilege(current_user,'lmm_bounties.records','UPDATE') AND NOT has_table_privilege(current_user,'lmm_bounties.records','DELETE,TRUNCATE')`,
		`NOT has_database_privilege(current_user,current_database(),'CREATE,TEMP') AND NOT has_schema_privilege(current_user,'public','CREATE,USAGE')`,
	}
	for _, check := range checks {
		if runtimeDB.QueryRow("SELECT "+check).Scan(&allowed) != nil || !allowed {
			t.Fatal("runtime exact minimum ACL rejected", check)
		}
	}
	key, e := os.ReadFile(f.KeyFile)
	if e != nil {
		t.Fatal("read private key")
	}
	store, e := toolmarket.OpenPostgresStore(ctx, runtimeDB, key)
	if e != nil {
		t.Fatal("actual encrypted snapshot cannot open as runtime")
	}
	defer store.Close()
	wrongKey := append([]byte(nil), key...)
	wrongKey[0] ^= 1
	if other, e := toolmarket.OpenPostgresStore(ctx, runtimeDB, wrongKey); e == nil {
		other.Close()
		t.Fatal("snapshot accepted wrong encryption key")
	}
	records, e := wallet.NewPostgresRecords(runtimeDB)
	if e != nil {
		t.Fatal("wallet records cannot open")
	}
	token, e := records.PutIfAbsent(ctx, 11, wallet.CreateInput{AmountCredits: 125, RequestKey: "fresh-install-wallet-key"}, strings.Repeat("a", 64))
	if e != nil || token != strings.Repeat("a", 64) {
		t.Fatal("runtime wallet durable token write failed")
	}
	if actual, e := records.Token(ctx, 11, "fresh-install-wallet-key"); e != nil || actual != token {
		t.Fatal("runtime wallet durable token read failed")
	}
	bounty, e := bounties.NewPostgres(ctx, runtimeDB, f.ExpectedDatabase)
	if e != nil {
		t.Fatal("runtime bounty store cannot open")
	}
	if e = bounty.Within(ctx, func(tx bounties.Tx) error {
		return tx.Put("projects", "fresh-project", bounties.Project{ID: "fresh-project", OwnerUserID: "11", Status: "draft"})
	}); e != nil {
		t.Fatal("runtime bounty write failed")
	}
	if e = bounty.Within(ctx, func(tx bounties.Tx) error {
		var project bounties.Project
		return tx.Get("projects", "fresh-project", &project)
	}); e != nil {
		t.Fatal("runtime bounty read failed")
	}
	if _, e = runtimeDB.Exec(`DELETE FROM lmm_bounties.records WHERE bucket='projects' AND id='fresh-project'`); e == nil {
		t.Fatal("runtime bounty hard DELETE was allowed")
	} else {
		denied(t, e)
	}
	if _, e = runtimeDB.Exec(`TRUNCATE lmm_bounties.records`); e == nil {
		t.Fatal("runtime bounty TRUNCATE was allowed")
	} else {
		denied(t, e)
	}
	if scalar(t, runtimeDB, `SELECT count(*) FROM lmm_bounties.records WHERE bucket='projects' AND id='fresh-project'`) != 1 {
		t.Fatal("rejected bounty DELETE changed original row")
	}
	var version int
	var ciphertext []byte
	if runtimeDB.QueryRow(`SELECT version,ciphertext FROM lmm_toolmarket.snapshot WHERE id=1`).Scan(&version, &ciphertext) != nil || version != 1 || len(ciphertext) < 28 {
		t.Fatal("missing encrypted initial snapshot")
	}
	if _, e = Install(ctx, f.config()); e == nil {
		t.Fatal("fresh installer accepted reentry")
	}
	var after []byte
	if runtimeDB.QueryRow(`SELECT ciphertext FROM lmm_toolmarket.snapshot WHERE id=1`).Scan(&after) != nil || string(after) != string(ciphertext) {
		t.Fatal("reentry replaced encrypted snapshot")
	}
	if _, e = runtimeDB.Exec(`CREATE TABLE public.runtime_escalation(id integer)`); e == nil {
		t.Fatal("runtime created DDL")
	} else {
		denied(t, e)
	}
	core := privateDB(t, f.CoreURLFile)
	denied(t, core.PingContext(ctx))
}

func TestFreshInstallClosedComponentContractsAndRuntimeStores(t *testing.T) {
	cases := fixtureCases(t)
	for _, label := range []string{"market", "assistant", "payments"} {
		t.Run(label, func(t *testing.T) {
			f := cases[label]
			mustInstall(t, f)
			db := privateDB(t, f.RuntimeURLFile)
			ctx := context.Background()
			switch label {
			case "market":
				key, e := os.ReadFile(f.KeyFile)
				if e != nil {
					t.Fatal("private key")
				}
				store, e := toolmarket.OpenPostgresStore(ctx, db, key)
				if e != nil {
					t.Fatal("plain toolmarket encrypted snapshot failed")
				}
				defer store.Close()
				if scalar(t, db, `SELECT count(*) FROM pg_namespace WHERE nspname IN('extension_wallet','lmm_bounties')`) != 0 {
					t.Fatal("plain toolmarket installed unrequested builtins")
				}
			case "assistant":
				if _, e := assistant.NewPostgres(ctx, db, f.ExpectedDatabase); e != nil {
					t.Fatal("assistant base runtime cannot open")
				}
				if _, e := assistant.NewDelegationPostgresJournal(ctx, db, f.ExpectedDatabase); e != nil {
					t.Fatal("assistant delegation runtime cannot open")
				}
				if _, e := assistant.NewAgentPostgresJournal(ctx, db, f.ExpectedDatabase); e != nil {
					t.Fatal("assistant agent runtime cannot open")
				}
				if scalar(t, db, `SELECT version FROM lmm_assistant.services_contract WHERE singleton`) != 1 {
					t.Fatal("assistant services increment missing")
				}
			case "payments":
				if _, e := payments.NewPostgres(ctx, db, f.ExpectedDatabase); e != nil {
					t.Fatal("payments runtime cannot open")
				}
			}
		})
	}
}

func TestFreshInstallRejectsOccupiedRuntimeRolePathsAndCoreConnect(t *testing.T) {
	cases := fixtureCases(t)
	for _, label := range []string{"occupied", "rolepath", "crossdb"} {
		t.Run(label, func(t *testing.T) {
			f := cases[label]
			if _, e := Install(context.Background(), f.config()); e == nil {
				t.Fatal("unsafe installation accepted", label)
			}
			db := privateDB(t, f.AdminURLFile)
			if label == "occupied" {
				if scalar(t, db, `SELECT count(*) FROM public.occupied`) != 0 {
					t.Fatal("preexisting object changed")
				}
				if scalar(t, db, `SELECT count(*) FROM pg_namespace WHERE nspname='lmm_toolmarket'`) != 0 {
					t.Fatal("occupied target partially installed")
				}
			} else {
				noApplicationObjects(t, db)
			}
		})
	}
}

func TestFreshInstallActualDDLFailureRollsBackEverySchemaSnapshotAndGrant(t *testing.T) {
	f := fixtureCases(t)["rollback"]
	owner := privateDB(t, f.OwnerURLFile)
	admin := privateDB(t, f.AdminURLFile)
	var tablespace string
	if owner.QueryRow(`SHOW default_tablespace`).Scan(&tablespace) != nil || tablespace != "pg_global" {
		t.Fatal("DDL failure injection missing on new installer LOGIN")
	}
	if _, e := Install(context.Background(), f.config()); e == nil {
		t.Fatal("invalid global tablespace did not fail actual table DDL")
	}
	noApplicationObjects(t, admin)
	if scalar(t, admin, `SELECT count(*) FROM information_schema.role_table_grants WHERE grantee=$1`, f.RuntimeRole) != 0 {
		t.Fatal("failed installation left runtime grants")
	}
	execPrivate(t, admin, "ALTER ROLE "+pq.QuoteIdentifier(f.InstallerRole)+" RESET default_tablespace")
	mustInstall(t, f)
}

func privateText(t *testing.T, db *sql.DB, query string, args ...any) string {
	t.Helper()
	var value string
	if db.QueryRow(query, args...).Scan(&value) != nil {
		t.Fatal("private fixture metadata query failed")
	}
	return value
}
func TestFreshInstallRejectsRuntimeOwnedPublicWithoutChangingAnyACL(t *testing.T) {
	f := fixtureCases(t)["publicowner"]
	admin := privateDB(t, f.AdminURLFile)
	snapshot := func() string {
		return privateText(t, admin, `SELECT jsonb_build_object('database_owner',d.datdba,'database_acl',d.datacl::text,'public_owner',n.nspowner,'public_acl',n.nspacl::text)::text FROM pg_database d CROSS JOIN pg_namespace n WHERE d.datname=current_database() AND n.nspname='public'`)
	}
	before := snapshot()
	if scalar(t, admin, `SELECT count(*) FROM pg_namespace WHERE nspname='public' AND pg_get_userbyid(nspowner)=$1`, f.RuntimeRole) != 1 {
		t.Fatal("runtime does not actually own empty public schema")
	}
	noApplicationObjects(t, admin)
	if _, e := Install(context.Background(), f.config()); e == nil {
		t.Fatal("runtime-owned public schema accepted")
	}
	noApplicationObjects(t, admin)
	if snapshot() != before {
		t.Fatal("rejected runtime-owned schema installation changed database or schema ownership/ACL")
	}
}
func TestFreshInstallDefaultAllPrivilegesCannotExpandCompiledRuntimeACL(t *testing.T) {
	f := fixtureCases(t)["defaultacl"]
	admin := privateDB(t, f.AdminURLFile)
	snapshot := func() string {
		return privateText(t, admin, `SELECT COALESCE(jsonb_agg(jsonb_build_object('role',defaclrole,'namespace',defaclnamespace,'type',defaclobjtype,'acl',defaclacl::text) ORDER BY defaclrole,defaclnamespace,defaclobjtype),'[]'::jsonb)::text FROM pg_default_acl`)
	}
	before := snapshot()
	if scalar(t, admin, `SELECT count(*) FROM pg_default_acl WHERE pg_get_userbyid(defaclrole)=$1 AND defaclobjtype IN('r','S','f')`, f.OwnerRole) != 3 {
		t.Fatal("table/sequence/function default ALL injection missing")
	}
	if scalar(t, admin, `SELECT count(*) FROM pg_default_acl d CROSS JOIN LATERAL aclexplode(d.defaclacl) a WHERE pg_get_userbyid(d.defaclrole)=$1 AND pg_get_userbyid(a.grantee)=$2`, f.OwnerRole, f.RuntimeRole) != 12 {
		t.Fatal("default ALL injection does not contain all 8 table,3 sequence,1 function privileges")
	}
	mustInstall(t, f)
	if after := snapshot(); after != before {
		t.Fatal("installer modified owner's existing default ACLs")
	}
	runtimeDB := privateDB(t, f.RuntimeURLFile)
	expected := map[string]string{
		"contract": "SELECT", "delegation_contract": "SELECT", "agent_contract": "SELECT", "services_contract": "SELECT", "preset_stats": "SELECT",
		"records": "SELECT,INSERT,UPDATE", "conversations": "SELECT,INSERT,UPDATE", "delegation_conversations": "SELECT,INSERT,UPDATE", "delegation_turns": "SELECT,INSERT,UPDATE", "agent_turns": "SELECT,INSERT,UPDATE", "site_issues": "SELECT,INSERT,UPDATE", "platform_support": "SELECT,INSERT,UPDATE", "platform_support_todos": "SELECT,INSERT,UPDATE",
		"history_messages": "SELECT,INSERT,DELETE",
		"turn_receipts":    "SELECT,INSERT", "service_operations": "SELECT,INSERT", "service_network_dispatch": "SELECT,INSERT", "service_confirmations": "SELECT,INSERT", "site_issue_events": "SELECT,INSERT", "platform_support_messages": "SELECT,INSERT",
		"memories": "SELECT,INSERT,UPDATE,DELETE", "profiles": "SELECT,INSERT,UPDATE,DELETE",
	}
	if scalar(t, runtimeDB, `SELECT count(*) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='lmm_assistant' AND c.relkind='r'`) != len(expected) {
		t.Fatal("assistant table ACL matrix is incomplete")
	}
	for table, privileges := range expected {
		for _, privilege := range []string{"SELECT", "INSERT", "UPDATE", "DELETE", "TRUNCATE", "REFERENCES", "TRIGGER", "MAINTAIN"} {
			var actual bool
			if runtimeDB.QueryRow(`SELECT has_table_privilege(current_user,$1,$2)`, "lmm_assistant."+table, privilege).Scan(&actual) != nil {
				t.Fatal("actual table privilege probe failed", table, privilege)
			}
			if actual != strings.Contains(","+privileges+",", ","+privilege+",") {
				t.Fatal("default ALL leaked or removed compiled table privilege", table, privilege, actual)
			}
		}
	}
	rows, e := runtimeDB.Query(`SELECT table_name,column_name FROM information_schema.columns WHERE table_schema='lmm_assistant' AND table_name IN('preset_stats','service_confirmations') ORDER BY table_name,ordinal_position`)
	if e != nil {
		t.Fatal("column privilege inventory failed")
	}
	type field struct{ table, column string }
	var columns []field
	for rows.Next() {
		var column field
		if rows.Scan(&column.table, &column.column) != nil {
			t.Fatal("column inventory scan")
		}
		columns = append(columns, column)
	}
	if rows.Err() != nil {
		t.Fatal("column inventory read")
	}
	rows.Close()
	if len(columns) < 5 {
		t.Fatal("column privilege matrix missing fields")
	}
	for _, column := range columns {
		var actual bool
		if runtimeDB.QueryRow(`SELECT has_column_privilege(current_user,$1,$2,'UPDATE')`, "lmm_assistant."+column.table, column.column).Scan(&actual) != nil {
			t.Fatal("column privilege probe failed")
		}
		want := column.table == "preset_stats" && column.column == "clicks" || column.table == "service_confirmations" && (column.column == "consumed_at" || column.column == "result")
		if actual != want {
			t.Fatal("default UPDATE leaked into restricted column", column.table, column.column, actual)
		}
	}
	var sequences, functions int
	if runtimeDB.QueryRow(`SELECT count(*),count(*) FILTER (WHERE has_sequence_privilege(current_user,c.oid,'USAGE') AND NOT has_sequence_privilege(current_user,c.oid,'SELECT,UPDATE')) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='lmm_assistant' AND c.relkind='S'`).Scan(&sequences, &functions) != nil || sequences == 0 || sequences != functions {
		t.Fatal("sequence default ALL leaked SELECT/UPDATE or removed required USAGE")
	}
	if runtimeDB.QueryRow(`SELECT count(*),count(*) FILTER(WHERE NOT has_function_privilege(current_user,p.oid,'EXECUTE')) FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname='lmm_assistant'`).Scan(&functions, &sequences) != nil || functions == 0 || functions != sequences {
		t.Fatal("function default EXECUTE leaked to runtime")
	}
	// Prove denials through actual commands as well as catalog privilege checks.
	for _, query := range []string{`DELETE FROM lmm_assistant.service_operations`, `UPDATE lmm_assistant.service_operations SET result=result`, `TRUNCATE lmm_assistant.service_operations`, `ALTER TABLE lmm_assistant.contract ADD COLUMN runtime_escape integer`, `SELECT setval('lmm_assistant.memories_id_seq',999)`, `SELECT lmm_assistant.services_immutable()`} {
		if _, e = runtimeDB.Exec(query); e == nil {
			t.Fatal("default ALL allowed forbidden operation", query)
		} else {
			denied(t, e)
		}
	}
	if snapshot() != before {
		t.Fatal("runtime probes modified original default ACL content")
	}
}
