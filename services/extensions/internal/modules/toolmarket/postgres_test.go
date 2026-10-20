// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package toolmarket

import (
	"bytes"
	"context"
	"database/sql"
	"errors"
	"net/url"
	"os"
	"os/exec"
	"strings"
	"sync"
	"testing"

	_ "github.com/lib/pq"
)

func toolmarketTestDB(t *testing.T) *sql.DB {
	return toolmarketDatabaseFile(t, "LMM_TOOLMARKET_TEST_DSN_FILE")
}
func toolmarketDatabaseFile(t *testing.T, env string) *sql.DB {
	t.Helper()
	path := os.Getenv(env)
	if path == "" {
		t.Skip("requires a new isolated toolmarket PostgreSQL database")
	}
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal("cannot read private PostgreSQL fixture")
	}
	u, err := url.Parse(strings.TrimSpace(string(data)))
	if err != nil || u.Scheme != "postgres" || (u.Hostname() != "127.0.0.1" && u.Hostname() != "localhost" && u.Hostname() != "::1") {
		t.Fatal("requires local isolated PostgreSQL fixture")
	}
	db, err := sql.Open("postgres", u.String())
	if err != nil {
		t.Fatal("cannot open fixture")
	}
	t.Cleanup(func() { db.Close() })
	return db
}

func TestPostgresStoreChild(t *testing.T) {
	if os.Getenv("LMM_TOOLMARKET_STORE_CHILD") != "1" {
		t.Skip("OS restart helper")
	}
	db := toolmarketTestDB(t)
	s, err := OpenPostgresStore(context.Background(), db, bytes.Repeat([]byte{71}, 32))
	if err != nil {
		t.Fatal(err)
	}
	defer s.Close()
	if err = s.update(func(d *database) error {
		if d.Secrets["synthetic"].Access != "synthetic-private-access" {
			return ErrConflict
		}
		d.Executions["os-restart"] = Execution{Owner: alice, State: "dispatched"}
		return nil
	}); err != nil {
		t.Fatal(err)
	}
}

func TestPostgresStorePersistenceConcurrentRollbackAndRestart(t *testing.T) {
	db := toolmarketTestDB(t)
	ownerDB := toolmarketDatabaseFile(t, "LMM_TOOLMARKET_TEST_OWNER_DSN_FILE")
	ctx := context.Background()
	// Refuse existing schema, even from a previous test run. Never overwrite or
	// remove a database to make acceptance pass.
	var exists bool
	if err := db.QueryRowContext(ctx, `SELECT EXISTS(SELECT 1 FROM pg_namespace WHERE nspname='lmm_toolmarket')`).Scan(&exists); err != nil || exists {
		t.Fatal("fixture is not fresh")
	}
	if _, err := ownerDB.ExecContext(ctx, PostgresSchema); err != nil {
		t.Fatal("cannot initialize isolated module schema")
	}
	key := bytes.Repeat([]byte{71}, 32)
	ownerStore, err := InitPostgresStore(ctx, ownerDB, key)
	if err != nil {
		t.Fatal(err)
	}
	if _, err = InitPostgresStore(ctx, ownerDB, key); !errors.Is(err, ErrConflict) {
		t.Fatal("reinitialized existing state", err)
	}
	if err = ownerStore.Close(); err != nil {
		t.Fatal(err)
	}
	var runtimeRole string
	if err = db.QueryRowContext(ctx, `SELECT current_user`).Scan(&runtimeRole); err != nil {
		t.Fatal("cannot resolve synthetic runtime role")
	}
	role := `"` + strings.ReplaceAll(runtimeRole, `"`, `""`) + `"`
	if _, err = ownerDB.ExecContext(ctx, `GRANT USAGE ON SCHEMA lmm_toolmarket TO `+role+`; GRANT SELECT,UPDATE ON lmm_toolmarket.snapshot TO `+role); err != nil {
		t.Fatal("cannot grant isolated runtime access")
	}
	s, err := OpenPostgresStore(ctx, db, key)
	if err != nil {
		t.Fatal(err)
	}
	if _, err = db.ExecContext(ctx, `CREATE TABLE lmm_toolmarket.runtime_must_not_install(id int)`); err == nil {
		t.Fatal("runtime can create schema objects")
	}
	if _, err = db.ExecContext(ctx, `INSERT INTO lmm_toolmarket.snapshot(id,version,ciphertext) SELECT 1,1,ciphertext FROM lmm_toolmarket.snapshot`); err == nil {
		t.Fatal("runtime can initialize module records")
	}
	if err = s.update(func(d *database) error {
		d.Secrets["synthetic"] = Secret{Access: "synthetic-private-access"}
		d.Grants = map[string]ExecutionGrant{"counter": {MaxCalls: 1000, State: "active"}}
		return nil
	}); err != nil {
		t.Fatal(err)
	}
	var ciphertext []byte
	if err = db.QueryRow(`SELECT ciphertext FROM lmm_toolmarket.snapshot WHERE id=1`).Scan(&ciphertext); err != nil {
		t.Fatal("cannot inspect stored ciphertext")
	}
	if bytes.Contains(ciphertext, []byte("synthetic-private-access")) {
		t.Fatal("credential stored in plaintext")
	}
	if _, err = OpenPostgresStore(ctx, db, bytes.Repeat([]byte{72}, 32)); !errors.Is(err, ErrInvalid) {
		t.Fatal("wrong encryption key accepted", err)
	}
	other, err := OpenPostgresStore(ctx, db, key)
	if err != nil {
		t.Fatal(err)
	}
	defer other.Close()
	var wg sync.WaitGroup
	errs := make(chan error, 40)
	for i := 0; i < 40; i++ {
		wg.Add(1)
		go func(i int) {
			defer wg.Done()
			target := s
			if i%2 == 0 {
				target = other
			}
			errs <- target.update(func(d *database) error { g := d.Grants["counter"]; g.UsedCalls++; d.Grants["counter"] = g; return nil })
		}(i)
	}
	wg.Wait()
	close(errs)
	for err := range errs {
		if err != nil {
			t.Fatal(err)
		}
	}
	if err = s.update(func(d *database) error { delete(d.Secrets, "synthetic"); return ErrDenied }); !errors.Is(err, ErrDenied) {
		t.Fatal(err)
	}
	// Real module persistence and a counted synthetic external MCP endpoint.
	// This exercises consent/dispatch storage, not Core money authority.
	mock := newMock(t)
	firstModule := testModule(t, mock)
	firstModule.store = s
	secondModule := testModule(t, mock)
	secondModule.store = other
	_, installation, _ := prepared(t, firstModule, mock, "none", "free")
	input := invocation(t, firstModule, alice, installation.ID, "pg-once")
	executionErrors := make(chan error, 16)
	for n := 0; n < 16; n++ {
		wg.Add(1)
		go func(n int) {
			defer wg.Done()
			target := firstModule
			if n%2 == 0 {
				target = secondModule
			}
			_, err := target.execute(ctx, alice, "alice", input)
			executionErrors <- err
		}(n)
	}
	wg.Wait()
	close(executionErrors)
	for err := range executionErrors {
		if err != nil && !errors.Is(err, ErrPending) {
			t.Fatal(err)
		}
	}
	if _, err := secondModule.execute(ctx, alice, "alice", input); err != nil {
		t.Fatal(err)
	}
	if mock.calls.Load() != 1 {
		t.Fatal("concurrent hosts repeated external tool side effect", mock.calls.Load())
	}
	if err = s.Close(); err != nil {
		t.Fatal(err)
	}
	child := exec.Command(os.Args[0], "-test.run=^TestPostgresStoreChild$", "-test.count=1")
	child.Env = append(os.Environ(), "LMM_TOOLMARKET_STORE_CHILD=1")
	if output, err := child.CombinedOutput(); err != nil {
		t.Fatalf("OS process reopen failed: %v %s", err, output)
	}
	if err = other.view(func(d database) error {
		if d.Grants["counter"].UsedCalls != 40 || d.Secrets["synthetic"].Access != "synthetic-private-access" || d.Executions["os-restart"].State != "dispatched" {
			return ErrConflict
		}
		return nil
	}); err != nil {
		t.Fatal("persistent state lost", err)
	}
	if err = s.view(func(database) error { return nil }); !errors.Is(err, ErrUnavailable) {
		t.Fatal("closed store accepted read", err)
	}
}
