/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import type { LogOtherData } from '../types'
import { searchUsageQuantities } from './search-usage'

const receipt = (usage: unknown): LogOtherData => ({
  search_billing: { provider: 'gemini_grounding_v1', unit: 'query', usage },
})

test('search counters distinguish missing, zero, charged unit, and unknown tokens', () => {
  const input = receipt({
    provider: 'gemini_grounding_v1',
    queries: 0,
    grounded_requests: 0,
  })
  assert.equal(searchUsageQuantities(input)[0]?.count, 0)
  assert.equal(searchUsageQuantities(receipt(null))[0]?.count, null)
  assert.equal(searchUsageQuantities(input, false)[0]?.count, null)
  assert.equal(
    searchUsageQuantities({ ...input, token_usage_available: false })[0]?.count,
    null
  )
  const positive = receipt({
    provider: 'gemini_grounding_v1',
    queries: 3,
    grounded_requests: 1,
  })
  assert.deepEqual(searchUsageQuantities(positive), [
    { label: 'Search queries', count: 3 },
  ])
  assert.deepEqual(
    searchUsageQuantities({
      search_billing: {
        provider: 'gemini_grounding_v1',
        unit: 'grounded_request',
        usage: {
          provider: 'gemini_grounding_v1',
          queries: 3,
          grounded_requests: 1,
        },
      },
    }),
    [{ label: 'Requests with search', count: 1 }]
  )
})

test('malformed, conflicting, and wrong-provider measurements are not presented as free', () => {
  for (const queries of [-1, 0.5, 10001, '1', NaN, Infinity]) {
    assert.equal(
      searchUsageQuantities(
        receipt({ provider: 'gemini_grounding_v1', queries })
      )[0]?.count,
      null
    )
  }
  for (const usage of [
    { provider: 'open_ai_responses_v1', queries: 1 },
    { provider: 'gemini_grounding_v1', queries: 1, grounded_requests: 0 },
    { provider: 'gemini_grounding_v1', queries: 1, x_posts_fetched: 1 },
  ])
    assert.equal(searchUsageQuantities(receipt(usage))[0]?.count, null)
})

test('xAI fetched posts and users remain separately measured dimensions', () => {
  const row: LogOtherData = {
    search_billing: {
      provider: 'xai_responses_v1',
      unit: 'call',
      x_posts_fetched_usd_per_unit: { numerator: '1', denominator: '1000' },
      x_users_fetched_usd_per_unit: { numerator: '0', denominator: '1' },
      usage: {
        provider: 'xai_responses_v1',
        calls: 2,
        x_posts_fetched: 8,
        x_users_fetched: 0,
      },
    },
  }
  assert.deepEqual(
    searchUsageQuantities(row).map((item) => item.count),
    [2, 8, 0]
  )
  assert.deepEqual(
    searchUsageQuantities(row, false).map((item) => item.count),
    [null, null, null]
  )
})
