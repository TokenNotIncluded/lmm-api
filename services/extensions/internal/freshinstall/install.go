// Package freshinstall installs compiled extension schemas explicitly into empty
// dedicated databases. It is never called by runtime composition.
package freshinstall

import (
	"context"
	"crypto/sha256"
	"database/sql"
	"encoding/hex"
	"errors"
	"net/url"
	"regexp"
	"strings"
	"time"

	"github.com/TokenNotIncluded/api.lmm.best/extensions/internal/coreclient"
	"github.com/TokenNotIncluded/api.lmm.best/extensions/internal/modules/assistant"
	"github.com/TokenNotIncluded/api.lmm.best/extensions/internal/modules/assistant_services"
	"github.com/TokenNotIncluded/api.lmm.best/extensions/internal/modules/bounties"
	"github.com/TokenNotIncluded/api.lmm.best/extensions/internal/modules/payments"
	"github.com/TokenNotIncluded/api.lmm.best/extensions/internal/modules/toolmarket"
	"github.com/TokenNotIncluded/api.lmm.best/extensions/internal/modules/wallet"
	"github.com/TokenNotIncluded/api.lmm.best/extensions/internal/postgresurl"
	"github.com/lib/pq"
)

type Config struct{ OwnerURLFile, ExpectedDatabase, OwnerRole, RuntimeRole, Component, EncryptionKeyFile string }
type Receipt struct {
	Component    string `json:"component"`
	Database     string `json:"database"`
	SchemaSHA256 string `json:"schema_sha256"`
}

var ErrRejected = errors.New("fresh extension installation rejected")
var ErrCommitUnknown = errors.New("extension installation commit outcome unknown; inspect the original database before any retry")
var name = regexp.MustCompile(`^[a-z][a-z0-9_]{0,62}$`)

type plan struct {
	schema, prefix string
	namespaces     []string
	grants         []string
	market         bool
}

func compiled(component string) (plan, error) {
	switch component {
	case "assistant":
		// Only this compiled increment has transaction wrappers. Remove its exact
		// framing so the complete installation remains one atomic transaction.
		services := assistant_services.Schema
		if strings.Count(services, "BEGIN;") != 1 || strings.Count(services, "COMMIT;") != 1 {
			return plan{}, ErrRejected
		}
		services = strings.Replace(services, "BEGIN;", "", 1)
		services = strings.Replace(services, "COMMIT;", "", 1)
		p := plan{prefix: "lmm_assistant", schema: assistant.Schema + "\n" + assistant.DelegationSchema + "\n" + assistant.AgentSchema + "\n" + services, namespaces: []string{"lmm_assistant"}}
		add := func(priv, tables string) { p.grants = append(p.grants, "GRANT "+priv+" ON "+tables+" TO ") }
		add("SELECT", "lmm_assistant.contract,lmm_assistant.delegation_contract,lmm_assistant.agent_contract,lmm_assistant.services_contract,lmm_assistant.preset_stats")
		add("SELECT,INSERT,UPDATE", "lmm_assistant.records,lmm_assistant.conversations,lmm_assistant.delegation_conversations,lmm_assistant.delegation_turns,lmm_assistant.agent_turns,lmm_assistant.site_issues,lmm_assistant.platform_support,lmm_assistant.platform_support_todos")
		add("SELECT,INSERT,DELETE", "lmm_assistant.history_messages")
		add("SELECT,INSERT", "lmm_assistant.turn_receipts,lmm_assistant.service_operations,lmm_assistant.service_network_dispatch,lmm_assistant.service_confirmations,lmm_assistant.site_issue_events,lmm_assistant.platform_support_messages")
		add("SELECT,INSERT,UPDATE,DELETE", "lmm_assistant.memories,lmm_assistant.profiles")
		add("UPDATE(clicks)", "lmm_assistant.preset_stats")
		add("UPDATE(consumed_at,result)", "lmm_assistant.service_confirmations")
		p.grants = append(p.grants, "GRANT USAGE ON ALL SEQUENCES IN SCHEMA lmm_assistant TO ")
		return p, nil
	case "toolmarket", "toolmarket-builtins":
		p := plan{prefix: "lmm_toolmarket", schema: toolmarket.PostgresSchema, namespaces: []string{"lmm_toolmarket"}, market: true, grants: []string{"GRANT SELECT,UPDATE ON lmm_toolmarket.snapshot TO "}}
		if component == "toolmarket-builtins" {
			p.schema += "\n" + wallet.Schema + "\n" + bounties.Schema
			p.namespaces = append(p.namespaces, "extension_wallet", "lmm_bounties")
			p.grants = append(p.grants, "GRANT SELECT,INSERT ON extension_wallet.transfer_links TO ", "GRANT SELECT ON lmm_bounties.contract TO ", "GRANT SELECT,INSERT,UPDATE ON lmm_bounties.records TO ")
		}
		return p, nil
	case "payments":
		return plan{prefix: "lmm_payments", schema: payments.Schema, namespaces: []string{"public"}, grants: []string{"GRANT SELECT ON public.payment_storage_guard TO ", "GRANT SELECT,INSERT,UPDATE ON public.payment_orders,public.payment_events TO "}}, nil
	}
	return plan{}, ErrRejected
}

func ownerURL(c Config, p plan) (string, error) {
	if !name.MatchString(c.OwnerRole) || !name.MatchString(c.RuntimeRole) || c.OwnerRole == c.RuntimeRole || !name.MatchString(c.ExpectedDatabase) || (c.ExpectedDatabase != p.prefix && !strings.HasPrefix(c.ExpectedDatabase, p.prefix+"_")) {
		return "", ErrRejected
	}
	raw, e := coreclient.ReadPrivateFile(c.OwnerURLFile, 8192)
	if e != nil {
		return "", ErrRejected
	}
	s := strings.TrimSpace(string(raw))
	u, e := url.Parse(s)
	if e != nil || u.User == nil || u.User.Username() == "" || (u.Scheme != "postgres" && u.Scheme != "postgresql") || u.Fragment != "" || strings.ContainsAny(s, "\r\n\x00") {
		return "", ErrRejected
	}
	unix, e := postgresurl.Unix(u)
	if e != nil {
		return "", ErrRejected
	}
	db, e := url.PathUnescape(strings.TrimPrefix(u.EscapedPath(), "/"))
	if e != nil || db != c.ExpectedDatabase {
		return "", ErrRejected
	}
	if unix {
		return s, nil
	}
	for k, v := range u.Query() {
		if len(v) != 1 || (k != "sslmode" && k != "sslrootcert" && k != "connect_timeout" && k != "application_name") {
			return "", ErrRejected
		}
	}
	return s, nil
}

// Install accepts only preprovisioned roles/databases and compiled schema inputs.
// A NOLOGIN database owner is reached through PostgreSQL's actual SET ROLE check.
func Install(ctx context.Context, c Config) (Receipt, error) {
	p, e := compiled(c.Component)
	if e != nil {
		return Receipt{}, e
	}
	dsn, e := ownerURL(c, p)
	if e != nil {
		return Receipt{}, e
	}
	var key []byte
	if p.market {
		key, e = coreclient.ReadPrivateFile(c.EncryptionKeyFile, 32)
		if e != nil || len(key) != 32 {
			return Receipt{}, ErrRejected
		}
	} else if c.EncryptionKeyFile != "" {
		return Receipt{}, ErrRejected
	}
	db, e := sql.Open("postgres", dsn)
	if e != nil {
		return Receipt{}, ErrRejected
	}
	defer db.Close()
	db.SetMaxOpenConns(1)
	ctx, cancel := context.WithTimeout(ctx, 30*time.Second)
	defer cancel()
	tx, e := db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelSerializable})
	if e != nil {
		return Receipt{}, ErrRejected
	}
	defer tx.Rollback()
	reject := func() (Receipt, error) { return Receipt{}, ErrRejected }
	if _, e = tx.ExecContext(ctx, `SET LOCAL lock_timeout='2s'; SET LOCAL statement_timeout='10s'`); e != nil {
		return reject()
	}
	var actual, session, owner string
	var unsafe bool
	e = tx.QueryRowContext(ctx, `SELECT current_database(),session_user,pg_get_userbyid(datdba),EXISTS(SELECT FROM pg_roles WHERE rolname IN(session_user,$1,$2) AND (rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication OR rolbypassrls)) FROM pg_database WHERE datname=current_database()`, c.OwnerRole, c.RuntimeRole).Scan(&actual, &session, &owner, &unsafe)
	if e != nil || actual != c.ExpectedDatabase || owner != c.OwnerRole || session == c.RuntimeRole || unsafe {
		return reject()
	}
	if _, e = tx.ExecContext(ctx, "SET LOCAL ROLE "+pq.QuoteIdentifier(c.OwnerRole)); e != nil {
		return reject()
	}
	// Reject runtime inheritance, owner membership, public CREATE/TEMP and any
	// access to a Core or other extension database; this installer never grants it.
	e = tx.QueryRowContext(ctx, `SELECT NOT EXISTS(SELECT FROM pg_roles WHERE rolname=$1 AND rolcanlogin) OR EXISTS(SELECT FROM pg_roles WHERE rolname<>$1 AND (pg_has_role($1,oid,'SET') OR pg_has_role($1,oid,'USAGE') OR pg_has_role($1,oid,'MEMBER'))) OR EXISTS(SELECT FROM pg_database WHERE datname<>current_database() AND (datname LIKE '%core%' OR datname LIKE 'lmm\_%' ESCAPE '\') AND has_database_privilege($1,oid,'CONNECT'))`, c.RuntimeRole).Scan(&unsafe)
	if e != nil || unsafe {
		return reject()
	}
	if _, e = tx.ExecContext(ctx, `SELECT pg_advisory_xact_lock(hashtextextended(current_database()||':extensions-fresh-install',0))`); e != nil {
		return reject()
	}
	// REVOKE cannot remove an owner's inherent rights. Only the database owner
	// (or PostgreSQL's database-owner pseudo-role) may own the empty public
	// schema; runtime ownership of any schema is forbidden before DDL begins.
	e = tx.QueryRowContext(ctx, `SELECT EXISTS(SELECT FROM pg_namespace WHERE nspowner=(SELECT oid FROM pg_roles WHERE rolname=$1)) OR EXISTS(SELECT FROM pg_namespace n JOIN pg_roles r ON r.oid=n.nspowner WHERE n.nspname='public' AND r.rolname NOT IN($2,'pg_database_owner'))`, c.RuntimeRole, c.OwnerRole).Scan(&unsafe)
	if e != nil || unsafe {
		return reject()
	}
	e = tx.QueryRowContext(ctx, `SELECT EXISTS(SELECT FROM pg_namespace WHERE nspname<>'public' AND nspname<>'information_schema' AND nspname NOT LIKE 'pg\_%' ESCAPE '\') OR EXISTS(SELECT FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public') OR EXISTS(SELECT FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname='public') OR EXISTS(SELECT FROM pg_type t JOIN pg_namespace n ON n.oid=t.typnamespace WHERE n.nspname='public') OR EXISTS(SELECT FROM pg_extension WHERE extname<>'plpgsql')`).Scan(&unsafe)
	if e != nil || unsafe {
		return reject()
	}
	if _, e = tx.ExecContext(ctx, p.schema); e != nil {
		return reject()
	}
	if p.market {
		if e = toolmarket.InitPostgresSnapshot(ctx, db, tx, key); e != nil {
			return reject()
		}
	}
	role := pq.QuoteIdentifier(c.RuntimeRole)
	if _, e = tx.ExecContext(ctx, "REVOKE ALL ON DATABASE "+pq.QuoteIdentifier(actual)+" FROM PUBLIC; REVOKE ALL ON DATABASE "+pq.QuoteIdentifier(actual)+" FROM "+role+"; GRANT CONNECT ON DATABASE "+pq.QuoteIdentifier(actual)+" TO "+role); e != nil {
		return reject()
	}
	if _, e = tx.ExecContext(ctx, "REVOKE ALL ON SCHEMA public FROM PUBLIC; REVOKE ALL ON SCHEMA public FROM "+role); e != nil {
		return reject()
	}
	for _, s := range p.namespaces {
		q := pq.QuoteIdentifier(s)
		// Owner default privileges may have granted this runtime role arbitrary
		// rights while the compiled objects were created. Reset their ACLs before
		// applying the exact grant list; leave the owner's default ACL unchanged.
		if _, e = tx.ExecContext(ctx, "REVOKE ALL ON SCHEMA "+q+" FROM PUBLIC; REVOKE ALL ON ALL TABLES IN SCHEMA "+q+" FROM PUBLIC; REVOKE ALL ON ALL SEQUENCES IN SCHEMA "+q+" FROM PUBLIC; REVOKE ALL ON ALL FUNCTIONS IN SCHEMA "+q+" FROM PUBLIC; REVOKE ALL ON ALL TABLES IN SCHEMA "+q+" FROM "+role+"; REVOKE ALL ON ALL SEQUENCES IN SCHEMA "+q+" FROM "+role+"; REVOKE ALL ON ALL FUNCTIONS IN SCHEMA "+q+" FROM "+role+"; REVOKE ALL ON SCHEMA "+q+" FROM "+role+"; GRANT USAGE ON SCHEMA "+q+" TO "+role); e != nil {
			return reject()
		}
	}
	for _, g := range p.grants {
		if _, e = tx.ExecContext(ctx, g+role); e != nil {
			return reject()
		}
	}
	if e = tx.Commit(); e != nil {
		return Receipt{}, ErrCommitUnknown
	}
	hash := sha256.Sum256([]byte(p.schema))
	return Receipt{Component: c.Component, Database: actual, SchemaSHA256: hex.EncodeToString(hash[:])}, nil
}
