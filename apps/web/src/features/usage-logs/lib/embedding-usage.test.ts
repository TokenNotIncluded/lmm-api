/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import type { LogOtherData } from '../types'
import { embeddingUsageQuantities } from './embedding-usage'

const receipt = (usage: unknown): LogOtherData => ({
  embedding_billing: { provider: 'ali_separate_input_v1', usage },
})
const measured = {
  provider: 'ali_separate_input_v1',
  text_tokens: 10,
  image_tokens: 20,
  total_input_tokens: 30,
}

test('embedding logs display measured modalities separately and preserve explicit zero', () => {
  assert.deepEqual(
    embeddingUsageQuantities(receipt(measured)).map((v) => v.count),
    [10, 20]
  )
  assert.deepEqual(
    embeddingUsageQuantities(
      receipt({ ...measured, image_tokens: 0, total_input_tokens: 10 })
    ).map((v) => v.count),
    [10, 0]
  )
  assert.deepEqual(embeddingUsageQuantities(null), [])
})

test('unknown, conflicting, malformed and unavailable embedding usage never displays zero', () => {
  for (const usage of [
    null,
    { ...measured, image_tokens: undefined },
    { ...measured, total_input_tokens: 29 },
    { ...measured, provider: 'other' },
    ...[-1, 0.5, '20', 16_000_001, NaN, Infinity].map((image_tokens) => ({
      ...measured,
      image_tokens,
    })),
  ]) {
    assert.deepEqual(
      embeddingUsageQuantities(receipt(usage)).map((v) => v.count),
      [null, null]
    )
  }
  assert.deepEqual(
    embeddingUsageQuantities(receipt(measured), false).map((v) => v.count),
    [null, null]
  )
  assert.deepEqual(
    embeddingUsageQuantities({
      ...receipt(measured),
      token_usage_available: false,
    }).map((v) => v.count),
    [null, null]
  )
})
