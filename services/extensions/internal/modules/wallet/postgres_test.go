package wallet

import (
	"context"
	"database/sql"
	"fmt"
	_ "github.com/lib/pq"
	"net/url"
	"os"
	"strings"
	"sync"
	"testing"
	"time"
)

// The private runner provisions a new pinned disposable container, supplies its
// DSN only in the environment, and removes only its own resources on success.
func TestOwnedPostgresDurableConcurrentRecords(t *testing.T) {
	raw := os.Getenv("WALLET_OWNED_TEST_DATABASE_URL")
	if raw == "" {
		t.Skip("requires newly provisioned wallet-owned PostgreSQL fixture")
	}
	u, e := url.Parse(raw)
	if e != nil || u.Hostname() != "127.0.0.1" || u.Path != "/postgres" {
		t.Fatal("private disposable fixture required")
	}
	admin, e := sql.Open("postgres", raw)
	if e != nil {
		t.Fatal("connect fixture")
	}
	defer admin.Close()
	name := fmt.Sprintf("wallet_records_%x", time.Now().UnixNano())
	if _, e = admin.Exec(`CREATE DATABASE ` + name); e != nil {
		t.Fatal("create own database")
	}
	defer func() {
		if _, e := admin.Exec(`DROP DATABASE ` + name + ` WITH (FORCE)`); e != nil {
			t.Error("own database cleanup failed")
		}
	}()
	u.Path = "/" + name
	db, e := sql.Open("postgres", u.String())
	if e != nil {
		t.Fatal("connect own database")
	}
	defer db.Close()
	if _, e = db.Exec(Schema); e != nil {
		t.Fatal("install wallet records schema")
	}
	store, e := NewPostgresRecords(db)
	if e != nil {
		t.Fatal(e)
	}
	in := CreateInput{AmountCredits: 500, RequestKey: "wallet-db-retry-0001"}
	var wg sync.WaitGroup
	tokens := make(chan string, 8)
	errs := make(chan error, 8)
	for n := 0; n < 8; n++ {
		wg.Add(1)
		go func(n int) {
			defer wg.Done()
			token, e := store.PutIfAbsent(context.Background(), 11, in, fmt.Sprintf("%064x", n+1))
			tokens <- token
			errs <- e
		}(n)
	}
	wg.Wait()
	close(tokens)
	close(errs)
	for e := range errs {
		if e != nil {
			t.Fatal(e)
		}
	}
	original := ""
	for token := range tokens {
		if original == "" {
			original = token
		}
		if token != original {
			t.Fatal("concurrent create changed bearer")
		}
	}
	reopened, e := NewPostgresRecords(db)
	if e != nil {
		t.Fatal(e)
	}
	token, e := reopened.PutIfAbsent(context.Background(), 11, in, strings.Repeat("f", 64))
	if e != nil || token != original {
		t.Fatal("reopen did not recover original bearer")
	}
	in.AmountCredits = 501
	if _, e = reopened.PutIfAbsent(context.Background(), 11, in, strings.Repeat("e", 64)); e != ErrConflict {
		t.Fatal("immutable amount not checked", e)
	}
	if _, e = store.Token(context.Background(), 12, in.RequestKey); e != ErrNotFound {
		t.Fatal("cross-owner bearer disclosed", e)
	}
	count := 0
	if e = db.QueryRow(`SELECT count(*) FROM extension_wallet.transfer_links`).Scan(&count); e != nil || count != 1 {
		t.Fatal("duplicate business rows")
	}
}
