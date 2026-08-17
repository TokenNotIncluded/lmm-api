package model

import (
	"errors"
	"fmt"
	"strings"
	"time"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"gorm.io/gorm"
)

const (
	FinanceEntryRevenue       = "revenue"
	FinanceEntryExpense       = "expense"
	FinanceEntryTokenCost     = "token_cost"
	FinanceEntryAdjustment    = "adjustment"
	FinanceDirectionCredit    = 1
	FinanceDirectionDebit     = -1
	FinanceCurrencyUSD        = "USD"
	FinanceSourceManual       = "manual"
	FinanceSourceTopUp        = "topup"
	FinanceSourceSubscription = "subscription"
	FinanceSourceUsage        = "usage"
	FinanceSourceRefund       = "refund"
)

var (
	ErrFinanceEntryInvalid    = errors.New("finance ledger entry is invalid")
	ErrFinanceEntryNotFound   = errors.New("finance ledger entry not found")
	ErrFinanceAlreadyReversed = errors.New("finance ledger entry has already been reversed")
)

// FinanceLedgerEntry is append-only. Corrections are represented by a new
// reversal entry; historical rows are never updated or deleted.
type FinanceLedgerEntry struct {
	Id              int64  `json:"id" gorm:"primaryKey"`
	EntryType       string `json:"entry_type" gorm:"type:varchar(32);index;not null"`
	Category        string `json:"category" gorm:"type:varchar(64);index;not null;default:''"`
	AmountMicros    int64  `json:"amount_micros" gorm:"not null"`
	Currency        string `json:"currency" gorm:"type:varchar(8);not null;default:'USD'"`
	Direction       int8   `json:"direction" gorm:"not null"`
	PaymentMethod   string `json:"payment_method" gorm:"type:varchar(64);index;not null;default:''"`
	PaymentProvider string `json:"payment_provider" gorm:"type:varchar(64);index;not null;default:''"`
	UserId          *int   `json:"user_id,omitempty" gorm:"index"`
	SourceType      string `json:"source_type" gorm:"type:varchar(32);index;not null"`
	SourceId        string `json:"source_id" gorm:"type:varchar(128);index;not null;default:''"`
	TokenUnits      int64  `json:"token_units" gorm:"not null;default:0"`
	Note            string `json:"note" gorm:"type:varchar(500);not null;default:''"`
	OccurredAt      int64  `json:"occurred_at" gorm:"not null;index"`
	CreatedAt       int64  `json:"created_at" gorm:"not null;index"`
	CreatedBy       int    `json:"created_by" gorm:"not null;index"`
	ReversalOfId    *int64 `json:"reversal_of_id,omitempty" gorm:"index"`
	IdempotencyKey  string `json:"-" gorm:"type:varchar(180);uniqueIndex"`
}

// FinancePaymentMethod controls which configured payment methods participate
// in revenue charts. It contains display metadata only, never gateway secrets.
type FinancePaymentMethod struct {
	Id             int64  `json:"id" gorm:"primaryKey"`
	Method         string `json:"method" gorm:"type:varchar(64);uniqueIndex;not null"`
	Label          string `json:"label" gorm:"type:varchar(100);not null"`
	Enabled        bool   `json:"enabled" gorm:"not null;default:true"`
	IncludeRevenue bool   `json:"include_revenue" gorm:"not null;default:true"`
	CreatedAt      int64  `json:"created_at" gorm:"not null"`
	UpdatedAt      int64  `json:"updated_at" gorm:"not null"`
	CreatedBy      int    `json:"created_by" gorm:"not null;index"`
}

func normalizeFinanceEntry(entry *FinanceLedgerEntry) error {
	if entry == nil || entry.AmountMicros <= 0 || entry.AmountMicros > 9_000_000_000_000_000 {
		return ErrFinanceEntryInvalid
	}
	entry.EntryType = strings.TrimSpace(entry.EntryType)
	entry.Category = strings.TrimSpace(entry.Category)
	entry.Currency = strings.ToUpper(strings.TrimSpace(entry.Currency))
	entry.PaymentMethod = strings.TrimSpace(entry.PaymentMethod)
	entry.PaymentProvider = strings.TrimSpace(entry.PaymentProvider)
	entry.SourceType = strings.TrimSpace(entry.SourceType)
	entry.SourceId = strings.TrimSpace(entry.SourceId)
	entry.Note = strings.TrimSpace(entry.Note)
	entry.IdempotencyKey = strings.TrimSpace(entry.IdempotencyKey)
	if entry.IdempotencyKey == "" {
		// The database keeps this column unique so supplied keys can make
		// webhook retries idempotent. An omitted key still means "no caller
		// supplied idempotency", not "reuse the empty string": generating an
		// internal key prevents the second unkeyed manual expense from failing
		// on that unique index while preserving explicit-key replay semantics.
		entry.IdempotencyKey = "finance:auto:" + common.NewRequestId()
	}
	if entry.Currency == "" {
		entry.Currency = FinanceCurrencyUSD
	}
	if entry.Direction != FinanceDirectionCredit && entry.Direction != FinanceDirectionDebit {
		return ErrFinanceEntryInvalid
	}
	if entry.SourceType == "" || entry.OccurredAt <= 0 || entry.CreatedBy <= 0 {
		return ErrFinanceEntryInvalid
	}
	if len([]rune(entry.Category)) > 64 || len([]rune(entry.Note)) > 500 || len(entry.Currency) > 8 {
		return ErrFinanceEntryInvalid
	}
	if entry.CreatedAt == 0 {
		entry.CreatedAt = time.Now().Unix()
	}
	switch entry.EntryType {
	case FinanceEntryRevenue, FinanceEntryExpense, FinanceEntryTokenCost, FinanceEntryAdjustment:
	default:
		return ErrFinanceEntryInvalid
	}
	return nil
}

// AppendFinanceLedgerEntry creates one immutable row. Idempotency keys make
// webhook/backfill retries safe and return the original row on replay.
func AppendFinanceLedgerEntry(entry *FinanceLedgerEntry) (*FinanceLedgerEntry, error) {
	persisted, _, err := AppendFinanceLedgerEntryIfNew(entry)
	return persisted, err
}

// AppendFinanceLedgerEntryIfNew is the same idempotent append operation, but
// also reports whether this call inserted the row. Webhook handlers use the
// flag to suppress duplicate side effects (for example, user-facing refund
// logs) when a provider retries an already recorded event.
func AppendFinanceLedgerEntryIfNew(entry *FinanceLedgerEntry) (*FinanceLedgerEntry, bool, error) {
	if err := normalizeFinanceEntry(entry); err != nil {
		return nil, false, err
	}
	if entry.IdempotencyKey != "" {
		var existing FinanceLedgerEntry
		if err := DB.Where("idempotency_key = ?", entry.IdempotencyKey).First(&existing).Error; err == nil {
			return &existing, false, nil
		} else if !errors.Is(err, gorm.ErrRecordNotFound) {
			return nil, false, err
		}
	}
	if err := DB.Create(entry).Error; err != nil {
		if entry.IdempotencyKey != "" {
			var existing FinanceLedgerEntry
			if lookupErr := DB.Where("idempotency_key = ?", entry.IdempotencyKey).First(&existing).Error; lookupErr == nil {
				return &existing, false, nil
			}
		}
		return nil, false, err
	}
	return entry, true, nil
}

// ReverseFinanceLedgerEntry appends a compensating row and never mutates the
// original. A row may only be reversed once.
func ReverseFinanceLedgerEntry(id int64, actorID int, now int64) (*FinanceLedgerEntry, error) {
	if id <= 0 || actorID <= 0 {
		return nil, ErrFinanceEntryInvalid
	}
	if now <= 0 {
		now = time.Now().Unix()
	}
	var reversal *FinanceLedgerEntry
	err := DB.Transaction(func(tx *gorm.DB) error {
		var original FinanceLedgerEntry
		if err := tx.First(&original, id).Error; err != nil {
			if errors.Is(err, gorm.ErrRecordNotFound) {
				return ErrFinanceEntryNotFound
			}
			return err
		}
		var count int64
		if err := tx.Model(&FinanceLedgerEntry{}).Where("reversal_of_id = ?", id).Count(&count).Error; err != nil {
			return err
		}
		if count > 0 {
			return ErrFinanceAlreadyReversed
		}
		copy := original
		copy.Id = 0
		// Keep the original category so aggregate queries can apply the
		// compensating direction without loading the parent row.
		copy.EntryType = original.EntryType
		copy.Direction = -original.Direction
		copy.SourceType = FinanceSourceManual
		copy.SourceId = fmt.Sprintf("reversal:%d", id)
		copy.Note = "Reversal of ledger entry " + fmt.Sprint(id)
		copy.OccurredAt = now
		copy.CreatedAt = now
		copy.CreatedBy = actorID
		copy.ReversalOfId = &id
		copy.IdempotencyKey = fmt.Sprintf("finance:reversal:%d", id)
		if err := tx.Create(&copy).Error; err != nil {
			return err
		}
		reversal = &copy
		return nil
	})
	return reversal, err
}
