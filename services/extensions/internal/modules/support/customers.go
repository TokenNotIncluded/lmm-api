package support

import (
	"context"
	"crypto/aes"
	"crypto/cipher"
	"crypto/rand"
	"encoding/base64"
	"errors"
	"fmt"
	"strings"
	"unicode/utf8"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
)

// CustomerRelationship is supplied only from the extension's persisted seller
// conversations. Core combines these identities with its own economic records.
type CustomerRelationship struct {
	BuyerID        int64  `json:"buyer_id"`
	ConversationID string `json:"conversation_id"`
}
type CustomerProjection interface {
	Customers(context.Context, string, int64, []CustomerRelationship, string, int, int) ([]WebCustomer, error)
	Customer(context.Context, string, int64, []CustomerRelationship, int64) (WebCustomer, error)
}
type CustomerIndex interface {
	CustomerRelationships(context.Context, int64) ([]CustomerRelationship, error)
}
type customerPorts struct {
	projection CustomerProjection
	aead       cipher.AEAD
}
type customerNote struct {
	Ciphertext string   `json:"ciphertext"`
	Tags       []string `json:"tags"`
	Revision   int64    `json:"revision"`
}

// WithCustomers returns a configured copy. The 32-byte installation key belongs
// only to this extension; no Core credential or database password is a note key.
func (s *Service) WithCustomers(projection CustomerProjection, key []byte) (*Service, error) {
	if s == nil || store.Missing(projection) || len(key) != 32 {
		return nil, store.ErrInvalid
	}
	if _, ok := s.repo.(CustomerIndex); !ok {
		return nil, store.ErrInvalid
	}
	block, err := aes.NewCipher(key)
	if err != nil {
		return nil, store.ErrInvalid
	}
	aead, err := cipher.NewGCM(block)
	if err != nil {
		return nil, store.ErrInvalid
	}
	out := *s
	out.customers = &customerPorts{projection: projection, aead: aead}
	return &out, nil
}
func noteNamespace(seller int64) string { return fmt.Sprintf("@web-customer:%d", seller) }
func noteID(buyer int64) string         { return fmt.Sprintf("buyer:%d", buyer) }
func noteAD(seller, buyer int64) []byte {
	return []byte(fmt.Sprintf("support-customer-note-v1:%d:%d", seller, buyer))
}
func (p *customerPorts) encrypt(seller, buyer int64, text string) (string, error) {
	nonce := make([]byte, p.aead.NonceSize())
	if _, err := rand.Read(nonce); err != nil {
		return "", store.ErrUnavailable
	}
	return base64.RawStdEncoding.EncodeToString(p.aead.Seal(nonce, nonce, []byte(text), noteAD(seller, buyer))), nil
}
func (p *customerPorts) decrypt(seller, buyer int64, text string) (string, error) {
	data, err := base64.RawStdEncoding.DecodeString(text)
	if err != nil || len(data) < p.aead.NonceSize() {
		return "", store.ErrUnavailable
	}
	plain, err := p.aead.Open(nil, data[:p.aead.NonceSize()], data[p.aead.NonceSize():], noteAD(seller, buyer))
	if err != nil {
		return "", store.ErrUnavailable
	}
	return string(plain), nil
}
func (s *Service) projectedCustomers(ctx context.Context, token string, actor int64, q string, offset, limit int) ([]WebCustomer, error) {
	if err := s.webActor(ctx, token, actor); err != nil {
		return nil, err
	}
	if s.customers == nil {
		return nil, store.ErrUnavailable
	}
	if !utf8.ValidString(q) || utf8.RuneCountInString(q) > 100 || offset < 0 || offset > 100000 || !store.Page(limit) {
		return nil, store.ErrInvalid
	}
	relations, err := s.repo.(CustomerIndex).CustomerRelationships(ctx, actor)
	if err != nil {
		return nil, err
	}
	return s.customers.projection.Customers(ctx, token, actor, relations, strings.TrimSpace(q), offset, limit)
}
func (s *Service) listCustomersWeb(ctx context.Context, token string, actor int64, q string, offset, limit int) ([]WebCustomer, error) {
	rows, err := s.projectedCustomers(ctx, token, actor, q, offset, limit)
	if err != nil {
		return nil, err
	}
	if rows == nil {
		rows = []WebCustomer{}
	}
	err = s.repo.Within(ctx, noteNamespace(actor), func(tx Tx) error {
		seen := map[int64]bool{}
		for i := range rows {
			row := &rows[i]
			if row.BuyerID <= 0 || row.BuyerID == actor || seen[row.BuyerID] || row.OrderCount < 0 || row.PaidQuota < 0 {
				return store.ErrUnavailable
			}
			seen[row.BuyerID] = true
			row.Note = ""
			row.Tags = []string{}
			row.Revision = 0
			var note customerNote
			err := tx.Get("commands", noteID(row.BuyerID), &note)
			if errors.Is(err, store.ErrNotFound) {
				continue
			}
			if err != nil {
				return err
			}
			row.Note, err = s.customers.decrypt(actor, row.BuyerID, note.Ciphertext)
			if err != nil {
				return err
			}
			row.Tags = append([]string{}, note.Tags...)
			row.Revision = note.Revision
		}
		return nil
	})
	return rows, err
}
func (s *Service) saveCustomerWeb(ctx context.Context, token string, actor, buyer int64, note string, tags []string, revision int64) error {
	if err := s.webActor(ctx, token, actor); err != nil {
		return err
	}
	if s.customers == nil {
		return store.ErrUnavailable
	}
	note = strings.TrimSpace(note)
	if buyer <= 0 || buyer == actor || revision < 0 || revision >= 9007199254740991 || !utf8.ValidString(note) || utf8.RuneCountInString(note) > 4000 || strings.ContainsRune(note, 0) || len(tags) > 10 {
		return store.ErrInvalid
	}
	clean := []string{}
	seen := map[string]bool{}
	for _, tag := range tags {
		tag = strings.TrimSpace(tag)
		if tag == "" || !utf8.ValidString(tag) || utf8.RuneCountInString(tag) > 32 || strings.ContainsAny(tag, "\x00\r\n") {
			return store.ErrInvalid
		}
		if !seen[tag] {
			clean = append(clean, tag)
			seen[tag] = true
		}
	}
	// Exact buyer lookup goes through Core, which rechecks the live Session and
	// confirms either a Core order or an extension-owned conversation relation.
	relations, err := s.repo.(CustomerIndex).CustomerRelationships(ctx, actor)
	if err != nil {
		return err
	}
	row, err := s.customers.projection.Customer(ctx, token, actor, relations, buyer)
	if err != nil {
		return err
	}
	if row.BuyerID != buyer {
		return store.ErrForbidden
	}
	ciphertext, err := s.customers.encrypt(actor, buyer, note)
	if err != nil {
		return err
	}
	return s.repo.Within(ctx, noteNamespace(actor), func(tx Tx) error {
		var old customerNote
		err := tx.Get("commands", noteID(buyer), &old)
		if errors.Is(err, store.ErrNotFound) {
			if revision != 0 {
				return store.ErrConflict
			}
		} else if err != nil {
			return err
		} else if old.Revision != revision {
			return store.ErrConflict
		}
		return tx.Put("commands", noteID(buyer), customerNote{Ciphertext: ciphertext, Tags: clean, Revision: revision + 1})
	})
}
