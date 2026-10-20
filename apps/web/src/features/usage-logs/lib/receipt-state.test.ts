/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { usageReceiptState } from './receipt-state'

test('incomplete native receipts preserve unknown reconciliation separately', () => {
  assert.deepEqual(
    usageReceiptState({ billing_basis: 'incomplete', state: 'reconcile' }),
    { status: 'unknown', label: 'Unknown', variant: 'warning' }
  )
  for (const state of ['reserved', 'streaming', 'released', undefined]) {
    assert.deepEqual(
      usageReceiptState({ billing_basis: 'incomplete', state }),
      { status: 'incomplete', label: 'Incomplete', variant: 'warning' }
    )
  }
})

test('metadata-only incomplete basis uses its captured request state', () => {
  assert.equal(
    usageReceiptState(
      {},
      { billing_basis: 'incomplete', request_state: 'reconcile' }
    )?.status,
    'unknown'
  )
  assert.equal(
    usageReceiptState(
      { state: 'streaming' },
      { billing_basis: 'incomplete', request_state: 'reconcile' }
    )?.status,
    'incomplete'
  )
})

test('state names and legacy errors do not invent a native incomplete receipt', () => {
  for (const state of [undefined, 'reconcile', 'reserved', 'settled']) {
    assert.equal(usageReceiptState({ state }), undefined)
  }
  assert.equal(
    usageReceiptState(
      { billing_basis: 'net_consumption', state: 'settled' },
      { billing_basis: 'net_consumption', request_state: 'settled' }
    ),
    undefined
  )
})
