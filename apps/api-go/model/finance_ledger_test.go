// Copyright (C) 2026 LIghtJUNction
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

package model

import (
	"testing"

	"github.com/stretchr/testify/require"
)

func TestAppendFinanceLedgerEntryGeneratesUniqueKeyWhenOmitted(t *testing.T) {
	db := setupConsoleActivationTestDB(t)
	require.NoError(t, db.AutoMigrate(&FinanceLedgerEntry{}))

	newExpense := func(note string) *FinanceLedgerEntry {
		return &FinanceLedgerEntry{
			EntryType:    FinanceEntryExpense,
			Category:     "hosting",
			AmountMicros: 1_000_000,
			Currency:     FinanceCurrencyUSD,
			Direction:    FinanceDirectionDebit,
			SourceType:   FinanceSourceManual,
			Note:         note,
			OccurredAt:   1,
			CreatedBy:    1,
		}
	}

	first, err := AppendFinanceLedgerEntry(newExpense("first"))
	require.NoError(t, err)
	second, err := AppendFinanceLedgerEntry(newExpense("second"))
	require.NoError(t, err)
	require.NotEmpty(t, first.IdempotencyKey)
	require.NotEmpty(t, second.IdempotencyKey)
	require.NotEqual(t, first.IdempotencyKey, second.IdempotencyKey)

	var count int64
	require.NoError(t, db.Model(&FinanceLedgerEntry{}).Count(&count).Error)
	require.Equal(t, int64(2), count)
}
