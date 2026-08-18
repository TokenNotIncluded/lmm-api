/*
Copyright (C) 2026 LIghtJUNction
*/
import assert from 'node:assert/strict'
import { describe, test } from 'node:test'

import {
  discountAfterAmountChange,
  discountCodeSavings,
} from './discount-state'

describe('discount state', () => {
  test('calculates coupon savings from the final quoted amount', () => {
    assert.equal(discountCodeSavings(90, 10), 10)
    assert.equal(discountCodeSavings(80, 20), 20)
    assert.equal(discountCodeSavings(90, null), 0)
    assert.equal(discountCodeSavings(0, 10), 0)
  })

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
