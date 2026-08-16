/*
Copyright (C) 2026 LIghtJUNction
*/
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { userNetRevenueMicros } from './user-finance-metrics'

test('reports user spend from receipts after refunds rather than platform cost', () => {
  assert.equal(userNetRevenueMicros(18_000_000, 5_500_000), 12_500_000)
  assert.equal(userNetRevenueMicros(0, 0), 0)
})
