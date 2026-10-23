package composition

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/support"
)

type customerPGAuthority struct{}

func (customerPGAuthority) Check(_ context.Context, token string, account store.Account, _ store.Permission) (int64, error) {
	if token == fixtureCredential(account.ID) && account.Kind == "personal" {
		return account.ID, nil
	}
	return 0, store.ErrForbidden
}

type customerPGDirectory struct{}

func (customerPGDirectory) Owner(context.Context, string) (store.Account, error) {
	return store.Account{Kind: "personal", ID: 202}, nil
}
func (customerPGDirectory) Parties(context.Context, string, string) (store.Account, store.Account, error) {
	return store.Account{Kind: "personal", ID: 101}, store.Account{Kind: "personal", ID: 202}, nil
}

// This fixture provides names/credits only. It cannot prove the separate Core
// projection, which has its own real ledger integration cases in Rust.
type customerPGProjection struct{}

func (customerPGProjection) Customers(_ context.Context, _ string, seller int64, relations []support.CustomerRelationship, q string, _, _ int) ([]support.WebCustomer, error) {
	if seller != 202 || len(relations) == 0 {
		return nil, store.ErrForbidden
	}
	for _, r := range relations {
		if r.BuyerID == 101 {
			return []support.WebCustomer{{BuyerID: 101, DisplayName: "synthetic profile", PaidQuota: 12345, OrderCount: 1, ConversationID: r.ConversationID}}, nil
		}
	}
	return nil, store.ErrForbidden
}

func (p customerPGProjection) Customer(ctx context.Context, token string, seller int64, relations []support.CustomerRelationship, buyer int64) (support.WebCustomer, error) {
	rows, err := p.Customers(ctx, token, seller, relations, "", 0, 1)
	if err != nil {
		return support.WebCustomer{}, err
	}
	if len(rows) != 1 || rows[0].BuyerID != buyer {
		return support.WebCustomer{}, store.ErrForbidden
	}
	return rows[0], nil
}

func TestSupportCustomersRealPostgresAcrossIndependentPools(t *testing.T) {
	root := os.Getenv("LMM_EXTENSION_ASSEMBLY_TEST_DIR")
	if root == "" {
		t.Skip("requires newly owned synthetic extension PostgreSQL fixture")
	}
	ctx := context.Background()
	cfg := StorageConfig{Component: "support", URLFile: filepath.Join(root, "support.url"), Database: "lmm_support_assembly_test"}
	first, err := openStorage(ctx, cfg)
	if err != nil {
		t.Fatal(err)
	}
	defer first.Close()
	second, err := openStorage(ctx, cfg)
	if err != nil {
		t.Fatal(err)
	}
	defer second.Close()
	repo, err := support.NewPostgres(first)
	if err != nil {
		t.Fatal(err)
	}
	peerRepo, err := support.NewPostgres(second)
	if err != nil {
		t.Fatal(err)
	}
	shop := "customer-pg-" + time.Now().UTC().Format("20060102150405.000000000")
	err = repo.Within(ctx, shop, func(tx support.Tx) error {
		return tx.Put("conversations", "customer-conversation", support.Conversation{ID: "customer-conversation", ShopID: shop, Buyer: store.Account{Kind: "personal", ID: 101}, Seller: store.Account{Kind: "personal", ID: 202}, CreatedAt: time.Now().UTC()})
	})
	if err != nil {
		t.Fatal(err)
	}
	key := []byte(strings.Repeat("k", 32))
	makeService := func(repo *support.Postgres) *support.Service {
		s, err := support.New(repo, customerPGAuthority{}, customerPGDirectory{})
		if err != nil {
			t.Fatal(err)
		}
		s, err = s.WithCustomers(customerPGProjection{}, key)
		if err != nil {
			t.Fatal(err)
		}
		return s
	}
	a, b := makeService(repo), makeService(peerRepo)
	// Use a separate buyer namespace each owned fixture; a prior invocation may
	// already have a note, so read its actual revision before the first update.
	rows, err := a.ListCustomersWeb(ctx, fixtureCredential(202), 202, "", 0, 30)
	if err != nil {
		t.Fatal(err)
	}
	revision := rows[0].Revision
	if err = a.SaveCustomerWeb(ctx, fixtureCredential(202), 202, 101, "SYNTHETIC_PRIVATE_PG_NOTE", []string{"private-tag"}, revision); err != nil {
		t.Fatal(err)
	}
	revision++
	var success atomic.Int64
	var wg sync.WaitGroup
	for i := 0; i < 16; i++ {
		wg.Add(1)
		go func(i int) {
			defer wg.Done()
			service := a
			if i%2 == 1 {
				service = b
			}
			err := service.SaveCustomerWeb(ctx, fixtureCredential(202), 202, 101, "concurrent committed", nil, revision)
			if err == nil {
				success.Add(1)
			} else if !errors.Is(err, store.ErrConflict) {
				t.Error(err)
			}
		}(i)
	}
	wg.Wait()
	if success.Load() != 1 {
		t.Fatal("cross-pool CAS", success.Load())
	}
	restarted := makeService(peerRepo)
	rows, err = restarted.ListCustomersWeb(ctx, fixtureCredential(202), 202, "", 0, 30)
	if err != nil || rows[0].Revision != revision+1 || rows[0].Note != "concurrent committed" {
		t.Fatal(rows, err)
	}
	var plaintext int
	if err = first.QueryRowContext(ctx, `SELECT count(*) FROM lmm_support.commands WHERE body::text LIKE '%SYNTHETIC_PRIVATE_PG_NOTE%' OR body::text LIKE '%concurrent committed%'`).Scan(&plaintext); err != nil || plaintext != 0 {
		t.Fatal("plaintext persisted", err)
	}
	wrong, err := restarted.WithCustomers(customerPGProjection{}, []byte(strings.Repeat("x", 32)))
	if err != nil {
		t.Fatal(err)
	}
	if _, err = wrong.ListCustomersWeb(ctx, fixtureCredential(202), 202, "", 0, 30); !errors.Is(err, store.ErrUnavailable) {
		t.Fatal("wrong key did not fail", err)
	}
}
