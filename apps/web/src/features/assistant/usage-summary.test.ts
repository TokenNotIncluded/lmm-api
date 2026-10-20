/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program. If not, see <https://www.gnu.org/licenses/>.

For commercial licensing, please contact support@quantumnous.com
*/
import assert from 'node:assert/strict'
import { describe, test } from 'node:test'

import { summarizeAssistantUsage } from './usage-summary'

describe('summarizeAssistantUsage', () => {
  test('aggregates totals and ranks duplicate model rows by spend', () => {
    const summary = summarizeAssistantUsage([
      {
        created_at: 1,
        model_name: 'model-a',
        count: 2,
        token_used: 50,
        quota: 2_000_000,
      },
      {
        created_at: 2,
        model_name: 'model-b',
        count: 4,
        token_used: 100,
        quota: 5_000_000,
      },
      {
        created_at: 3,
        model_name: 'model-a',
        count: 1,
        token_used: 25,
        quota: 1_000_000,
      },
    ])

    assert.equal(summary.requests, 7)
    assert.equal(summary.tokens, 175)
    assert.equal(summary.credits, 8_000_000)
    assert.deepEqual(
      summary.models.map((model) => ({
        model: model.model,
        requests: model.requests,
        credits: model.credits,
      })),
      [
        { model: 'model-b', requests: 4, credits: 5_000_000 },
        { model: 'model-a', requests: 3, credits: 3_000_000 },
      ]
    )
    assert.equal(summary.models[0]?.sharePercent, 62.5)
  })

  test('ignores invalid negative counters and falls back to token share', () => {
    const summary = summarizeAssistantUsage([
      {
        created_at: 1,
        model_name: 'model-a',
        count: -1,
        token_used: 30,
        quota: Number.NaN,
      },
      {
        created_at: 2,
        model_name: 'model-b',
        count: 2,
        token_used: 70,
        quota: -10,
      },
    ])

    assert.equal(summary.requests, 2)
    assert.equal(summary.tokens, 100)
    assert.equal(summary.credits, 0)
    assert.equal(summary.models[0]?.model, 'model-b')
    assert.equal(summary.models[0]?.sharePercent, 70)
  })
})

test('assistant image-only usage retains null tokens, image counts and sponsor funding', () => {
  const summary = summarizeAssistantUsage([
    {
      created_at: 1,
      model_name: 'image',
      count: 1,
      token_used: null,
      quota: 2000,
      output_image_count: 2,
      platform_sponsored: true,
      token_count_scope: 'settled_text_requests',
    },
    {
      created_at: 2,
      model_name: 'image',
      count: 1,
      token_used: null,
      quota: 3000,
      output_image_count: 3,
      platform_sponsored: false,
      token_count_scope: 'settled_text_requests',
    },
  ])
  assert.equal(summary.tokens, null)
  assert.equal(summary.outputImages, 5)
  assert.equal(summary.credits, 5000)
  assert.equal(summary.sponsoredCredits, 2000)
  assert.equal(summary.sponsoredRequests, 1)
  assert.equal(summary.tokenCountScope, 'settled_text_requests')
  assert.equal(summary.models[0]?.tokens, null)
})

test('assistant preserves unknown credits and a true zero text measurement', () => {
  const summary = summarizeAssistantUsage([
    {
      created_at: 1,
      model_name: 'image',
      count: 1,
      token_used: null,
      quota: null,
      output_image_count: 1,
      platform_sponsored: true,
    },
    {
      created_at: 2,
      model_name: 'text-zero',
      count: 1,
      token_used: 0,
      quota: 0,
    },
  ])
  assert.equal(summary.tokens, 0)
  assert.equal(summary.credits, null)
  assert.equal(summary.sponsoredCredits, null)
  assert.equal(summary.models.length, 2)
  assert.equal(
    summary.models.find((row) => row.model === 'text-zero')?.tokens,
    0
  )
})
