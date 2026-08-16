/*
Copyright (C) 2026 LIghtJUNction
*/
import assert from 'node:assert/strict'
import { describe, test } from 'node:test'

import { discountAfterAmountChange } from './discount-state'

describe('discount state', () => {
  test('invalidates a validated code when the credited amount changes', () => {
    assert.deepEqual(
      discountAfterAmountChange({ code: 'WEEKLY10', percent: 10 }, 100, 20),
      { code: '', percent: null }
    )
  })

  test('preserves the state when the amount is unchanged or no code was applied', () => {
    const applied = { code: 'WEEKLY10', percent: 10 }
    assert.deepEqual(discountAfterAmountChange(applied, 100, 100), applied)
    assert.deepEqual(
      discountAfterAmountChange({ code: '', percent: null }, 100, 20),
      { code: '', percent: null }
    )
  })
})
