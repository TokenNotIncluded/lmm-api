/*
Copyright (C) 2026 LIghtJUNction
*/
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { paymentMethodSummary } from './payment-method-metrics'

test('reconciles a payment method from revenue and refund metrics', () => {
  const summary = paymentMethodSummary(
    'waffo_pancake',
    [
      {
        method: 'waffo_pancake',
        provider: 'pancake-card',
        amount_micros: 12_000_000,
        orders: 2,
        users: 2,
        token_units: 0,
      },
      {
        method: 'waffo_pancake',
        provider: 'pancake-bank',
        amount_micros: 3_000_000,
        orders: 1,
        users: 1,
        token_units: 0,
      },
      {
        method: 'stripe',
        provider: 'stripe',
        amount_micros: 99_000_000,
        orders: 1,
        users: 1,
        token_units: 0,
      },
    ],
    [
      {
        method: 'waffo_pancake',
        provider: 'pancake-card',
        amount_micros: 4_000_000,
        orders: 1,
        users: 1,
        token_units: 0,
      },
    ]
  )

  assert.deepEqual(summary, {
    revenueMicros: 15_000_000,
    refundMicros: 4_000_000,
    netRevenueMicros: 11_000_000,
  })
})

test('returns a zero summary when the method has no financial activity', () => {
  assert.deepEqual(paymentMethodSummary('free', undefined, undefined), {
    revenueMicros: 0,
    refundMicros: 0,
    netRevenueMicros: 0,
  })
})
