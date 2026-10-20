/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import type { QuotaDataItem } from '../types'
import { calculateDashboardStats, safeDivide } from './stats'

const image: QuotaDataItem = {
  created_at: 1,
  count: 1,
  quota: 2000,
  token_used: null,
  output_image_count: 2,
  token_count_scope: 'settled_text_requests',
  platform_sponsored: true,
}
const text: QuotaDataItem = {
  created_at: 1,
  count: 2,
  quota: 7,
  token_used: 193,
  output_image_count: 0,
  token_count_scope: 'settled_text_requests',
}

test('image-only usage preserves requests, consumption and images without inventing token zero', () => {
  assert.deepEqual(calculateDashboardStats([image]), {
    totalCount: 1,
    totalQuota: 2000,
    totalTokens: null,
    totalOutputImages: 2,
    tokenCountScope: 'settled_text_requests',
    hasOutputImages: true,
    hasPlatformSponsoredUsage: true,
  })
})

test('mixed settled usage retains known text token totals in either bucket order', () => {
  for (const rows of [
    [image, text],
    [text, image],
  ]) {
    const stats = calculateDashboardStats(rows)
    assert.equal(stats.totalTokens, 193)
    assert.equal(stats.totalCount, 3)
    assert.equal(stats.totalQuota, 2007)
    assert.equal(stats.totalOutputImages, 2)
    assert.equal(stats.tokenCountScope, 'settled_text_requests')
  }
})

test('measured zero and a successful empty result remain known zero', () => {
  assert.equal(
    calculateDashboardStats([{ ...text, token_used: 0 }]).totalTokens,
    0
  )
  assert.deepEqual(calculateDashboardStats([]), {
    totalCount: 0,
    totalQuota: 0,
    totalTokens: 0,
    totalOutputImages: 0,
    tokenCountScope: undefined,
    hasOutputImages: false,
    hasPlatformSponsoredUsage: false,
  })
})

test('unexplained missing or invalid tokens leave the total unavailable without dropping other metrics', () => {
  const unknownRows: QuotaDataItem[] = [
    { ...image, token_used: undefined },
    { ...image, token_count_scope: undefined },
    { ...image, output_image_count: undefined },
    { ...image, output_image_count: null },
    { ...image, output_image_count: 0 },
    { ...text, token_used: Number.NaN },
    { ...text, token_used: Number.POSITIVE_INFINITY },
    { ...text, token_used: -1 },
  ]
  for (const row of unknownRows) {
    const stats = calculateDashboardStats([row, text])
    assert.equal(stats.totalTokens, null)
    assert.equal(stats.totalCount, (row.count ?? 0) + 2)
    assert.equal(stats.totalQuota, (row.quota ?? 0) + 7)
  }
})

test('legacy numeric token totals stay available without claiming a native scope or image zero', () => {
  const stats = calculateDashboardStats([
    { created_at: 1, count: 1, quota: 2, token_used: 5 },
    text,
  ])
  assert.equal(stats.totalTokens, 198)
  assert.equal(stats.totalOutputImages, null)
  assert.equal(stats.tokenCountScope, undefined)
})

test('unknown output images do not discard independently known token or consumption totals', () => {
  const stats = calculateDashboardStats([
    { ...text, output_image_count: null },
    text,
  ])
  assert.equal(stats.totalOutputImages, null)
  assert.equal(stats.totalTokens, 386)
  assert.equal(stats.totalQuota, 14)
  assert.equal(stats.totalCount, 4)
})

test('unknown legacy consumption remains unavailable without losing measured usage', () => {
  for (const quota of [null, undefined, Number.NaN, 0.5]) {
    const stats = calculateDashboardStats([{ ...text, quota }, text])
    assert.equal(stats.totalQuota, null)
    assert.equal(stats.totalCount, 4)
    assert.equal(stats.totalTokens, 386)
    assert.equal(stats.totalOutputImages, 0)
  }
  assert.equal(calculateDashboardStats([{ ...text, quota: 0 }]).totalQuota, 0)
  assert.equal(calculateDashboardStats([{ ...text, quota: -2 }]).totalQuota, -2)
})

test('summed quantities beyond the exact number boundary stay unavailable', () => {
  const stats = calculateDashboardStats([
    {
      ...text,
      quota: Number.MAX_SAFE_INTEGER,
      token_used: Number.MAX_SAFE_INTEGER,
      output_image_count: Number.MAX_SAFE_INTEGER,
    },
    { ...text, quota: 1, token_used: 1, output_image_count: 1 },
  ])
  assert.equal(stats.totalQuota, null)
  assert.equal(stats.totalTokens, null)
  assert.equal(stats.totalOutputImages, null)
  assert.equal(stats.totalCount, 4)
})

test('safeDivide keeps its numeric caller contract', () => {
  assert.equal(safeDivide(193, 2), 96.5)
  assert.equal(safeDivide(1, 0), 0)
  assert.equal(safeDivide(1, 3, 2), 0.33)
})
