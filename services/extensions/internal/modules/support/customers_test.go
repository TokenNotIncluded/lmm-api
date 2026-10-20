package support

import (
	"context"
	"encoding/json"
	"errors"
	"strings"
	"sync"
	"sync/atomic"
	"testing"

	"github.com/TokenNotIncluded/api.lmm.best/extensions/internal/modules/store"
)

type customerRepo struct{ *webMemoryRepo }

func (r *customerRepo) CustomerRelationships(ctx context.Context, seller int64) ([]CustomerRelationship, error) {
	r.mu.Lock()
	defer r.mu.Unlock()
	out := []CustomerRelationship{}
	for _, rows := range r.data {
		for key, raw := range rows {
			if !strings.HasPrefix(key, "conversations/") {
				continue
			}
			var c Conversation
			if err := json.Unmarshal(raw, &c); err != nil {
				return nil, err
			}
			if c.Seller.Kind == "personal" && c.Seller.ID == seller && c.Buyer.Kind == "personal" {
				out = append(out, CustomerRelationship{c.Buyer.ID, c.ID})
			}
		}
	}
	return out, ctx.Err()
}

type customerProjection struct {
	calls atomic.Int64
	err   error
}

func (p *customerProjection) Customers(_ context.Context, token string, seller int64, relations []CustomerRelationship, q string, offset, limit int) ([]WebCustomer, error) {
	p.calls.Add(1)
	if p.err != nil {
		return nil, p.err
	}
	if token != sellerToken || seller != 1 {
		return nil, store.ErrForbidden
	}
	if len(relations) != 1 || relations[0].BuyerID != 2 {
		return nil, store.ErrUnavailable
	}
	if q == "id:999" {
		return []WebCustomer{}, nil
	}
	return []WebCustomer{{BuyerID: 2, DisplayName: "Core public name", PaidQuota: 500000, OrderCount: 1, ConversationID: relations[0].ConversationID}}, nil
}
func (p *customerProjection) Customer(ctx context.Context, token string, seller int64, relations []CustomerRelationship, buyer int64) (WebCustomer, error) {
	rows, err := p.Customers(ctx, token, seller, relations, "", 0, 1)
	if err != nil {
		return WebCustomer{}, err
	}
	if buyer != 2 || len(rows) != 1 {
		return WebCustomer{}, store.ErrForbidden
	}
	return rows[0], nil
}
func newCustomerFixture(t *testing.T) (*Service, *customerRepo, *customerProjection, *authority) {
	t.Helper()
	f := newWebFixture(t)
	_, err := f.s.OpenWeb(context.Background(), buyerToken, 2, "product", "")
	must(t, err)
	repo := &customerRepo{f.repo}
	s, err := New(repo, f.auth, f.dir)
	must(t, err)
	projection := &customerProjection{}
	s, err = s.WithCustomers(projection, []byte(strings.Repeat("k", 32)))
	must(t, err)
	return s, repo, projection, f.auth
}
func TestCustomersPersistEncryptedNotesAndRejectCrossSellerRevision(t *testing.T) {
	s, repo, projection, auth := newCustomerFixture(t)
	ctx := context.Background()
	must(t, s.SaveCustomerWeb(ctx, sellerToken, 1, 2, " PRIVATE_NOTE ", []string{" tag ", "tag"}, 0))
	rows, err := s.ListCustomersWeb(ctx, sellerToken, 1, "", 0, 30)
	must(t, err)
	if len(rows) != 1 || rows[0].Note != "PRIVATE_NOTE" || rows[0].Revision != 1 || len(rows[0].Tags) != 1 || rows[0].PaidQuota != 500000 || rows[0].DisplayName != "Core public name" {
		t.Fatal(rows)
	}
	repo.mu.Lock()
	raw := repo.data[noteNamespace(1)]["commands/"+noteID(2)]
	repo.mu.Unlock()
	if strings.Contains(string(raw), "PRIVATE_NOTE") {
		t.Fatal("plaintext note persisted")
	}
	if err = s.SaveCustomerWeb(ctx, sellerToken, 1, 2, "lost update", nil, 0); !errors.Is(err, store.ErrConflict) {
		t.Fatal(err)
	}
	if err = s.SaveCustomerWeb(ctx, sellerToken, 1, 999, "unrelated", nil, 0); !errors.Is(err, store.ErrForbidden) {
		t.Fatal(err)
	}
	// Restart with the same key reads the durable encrypted row; a wrong key
	// fails explicitly instead of silently replacing it with an empty note.
	restarted, err := s.WithCustomers(projection, []byte(strings.Repeat("k", 32)))
	must(t, err)
	rows, err = restarted.ListCustomersWeb(ctx, sellerToken, 1, "", 0, 30)
	must(t, err)
	if rows[0].Note != "PRIVATE_NOTE" {
		t.Fatal(rows)
	}
	wrong, err := s.WithCustomers(projection, []byte(strings.Repeat("x", 32)))
	must(t, err)
	if _, err = wrong.ListCustomersWeb(ctx, sellerToken, 1, "", 0, 30); !errors.Is(err, store.ErrUnavailable) {
		t.Fatal(err)
	}
	before := projection.calls.Load()
	auth.revoked.Store(true)
	if _, err = s.ListCustomersWeb(ctx, sellerToken, 1, "", 0, 30); !errors.Is(err, store.ErrForbidden) || projection.calls.Load() != before {
		t.Fatal(err)
	}
}
func TestCustomersConcurrentCASRollbackAndCiphertextBinding(t *testing.T) {
	s, repo, _, _ := newCustomerFixture(t)
	ctx := context.Background()
	must(t, s.SaveCustomerWeb(ctx, sellerToken, 1, 2, "initial", nil, 0))
	var success atomic.Int64
	var wg sync.WaitGroup
	for i := 0; i < 16; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			err := s.SaveCustomerWeb(ctx, sellerToken, 1, 2, "concurrent", nil, 1)
			if err == nil {
				success.Add(1)
			} else if !errors.Is(err, store.ErrConflict) {
				t.Error(err)
			}
		}()
	}
	wg.Wait()
	if success.Load() != 1 {
		t.Fatal(success.Load())
	}
	repo.failCommit.Store(true)
	if err := s.SaveCustomerWeb(ctx, sellerToken, 1, 2, "failed", nil, 2); err == nil {
		t.Fatal("commit failure ignored")
	}
	rows, err := s.ListCustomersWeb(ctx, sellerToken, 1, "", 0, 30)
	must(t, err)
	if rows[0].Revision != 2 || rows[0].Note != "concurrent" {
		t.Fatal(rows)
	}
	encrypted, err := s.customers.encrypt(1, 2, "private")
	must(t, err)
	if _, err = s.customers.decrypt(2, 1, encrypted); !errors.Is(err, store.ErrUnavailable) {
		t.Fatal("seller/buyer AAD ignored", err)
	}
}
