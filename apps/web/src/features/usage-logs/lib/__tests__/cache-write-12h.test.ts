/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { cacheWrite12hQuantity } from '../usage-quantity'

test('12h log quantity is a measured subset and preserves explicit zero', () => {
  assert.equal(
    cacheWrite12hQuantity({
      cache_creation_tokens: 7000,
      cache_creation_tokens_12h: 4000,
    }),
    4000
  )
  assert.equal(
    cacheWrite12hQuantity({
      cache_creation_tokens: 7000,
      cache_creation_tokens_12h: 0,
    }),
    0
  )
})

test('missing, malformed, excessive and incomplete quantities stay unknown', () => {
  for (const other of [
    null,
    {},
    { cache_creation_tokens: 7000 },
    { cache_creation_tokens_12h: 4000 },
    { cache_creation_tokens: 7000, cache_creation_tokens_12h: null },
    { cache_creation_tokens: 7000, cache_creation_tokens_12h: -1 },
    { cache_creation_tokens: 7000, cache_creation_tokens_12h: 1.5 },
    { cache_creation_tokens: 7000, cache_creation_tokens_12h: 7001 },
    {
      cache_creation_tokens: 7000,
      cache_creation_tokens_12h: 1,
      token_usage_available: false,
    },
    {
      cache_creation_tokens: Number.MAX_SAFE_INTEGER + 1,
      cache_creation_tokens_12h: 1,
    },
  ])
    assert.equal(cacheWrite12hQuantity(other), null)
})
