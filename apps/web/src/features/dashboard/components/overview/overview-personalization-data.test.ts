/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import type { QuotaDataItem } from '@/features/dashboard/types'

import {
  buildOverviewUsage,
  expandOverviewGreeting,
  overviewLanguage,
  overviewUsageRange,
  parseOverviewPreference,
  validGreetingTemplate,
  type OverviewMetric,
} from './overview-personalization-data'

test('greeting uses literal one-pass variables and locale fallback', () => {
  assert.equal(
    expandOverviewGreeting('HI,$name, $time; $$ $balance', {
      $name: '$time',
      $time: '12:30',
      $balance: '0.25',
    }),
    'HI,$time, 12:30; $ 0.25'
  )
  assert.ok(validGreetingTemplate('HI,$name,现在是$time'))
  assert.ok(validGreetingTemplate(''))
  assert.equal(validGreetingTemplate('$secret'), false)
  assert.equal(validGreetingTemplate('a'.repeat(513)), false)
  assert.equal(overviewLanguage('zh-Hant-TW'), 'zh-TW')
  assert.equal(overviewLanguage('zhCN'), 'zh')
  assert.equal(overviewLanguage('zhTW'), 'zh-TW')
  assert.equal(overviewLanguage('en-US'), 'en')
  assert.equal(overviewLanguage('xx'), 'en')
  assert.throws(() =>
    parseOverviewPreference({
      revision: 0,
      updated_at: 0,
      templates: { en: '$secret' },
    })
  )
  assert.throws(() =>
    parseOverviewPreference({ revision: -1, updated_at: 0, templates: {} })
  )
})
test('usage totals use recorded values and fold the remainder without loss', () => {
  const now = new Date(2026, 9, 9, 19, 0, 0)
  const range = overviewUsageRange(7, now)
  const rows = Array.from({ length: 7 }, (_, i) => ({
    created_at: range.end_timestamp,
    model_name: `model-${i}`,
    count: i + 1,
    token_used: (i + 1) * 10,
    quota: (i + 1) * 100,
  }))
  const data = buildOverviewUsage(
    rows,
    'requests',
    range.start_timestamp,
    range.end_timestamp,
    'en',
    'Other',
    'Unknown'
  )
  assert.equal(data.labels.length, 7)
  assert.equal(data.total, 28)
  assert.equal(data.models.length, 6)
  assert.equal(
    data.models.reduce((total, row) => total + row[1], 0),
    28
  )
  assert.equal(
    buildOverviewUsage(
      [],
      'tokens',
      range.start_timestamp,
      range.end_timestamp,
      'en',
      'Other',
      'Unknown'
    ).total,
    0
  )
  assert.throws(() =>
    buildOverviewUsage(
      [{ created_at: range.end_timestamp }],
      'tokens',
      range.start_timestamp,
      range.end_timestamp,
      'en',
      'Other',
      'Unknown'
    )
  )
})

test('usage date formatting accepts the actual Chinese interface codes', () => {
  const range = overviewUsageRange(7, new Date(2026, 9, 9, 19, 0, 0))
  for (const locale of ['zhCN', 'zhTW']) {
    const result = buildOverviewUsage(
      [{ created_at: range.end_timestamp, count: 42, model_name: 'test' }],
      'requests',
      range.start_timestamp,
      range.end_timestamp,
      locale,
      '其他',
      '未知'
    )
    assert.equal(result.total, 42)
    assert.equal(result.labels.length, 7)
    assert.ok(result.labels.some((label) => label.includes('月')))
  }
})

const usageTime = new Date(2026, 9, 9, 19, 0, 0)
const usageRange = overviewUsageRange(7, usageTime)
function usageFor(rows: QuotaDataItem[], metric: OverviewMetric) {
  return buildOverviewUsage(
    rows,
    metric,
    usageRange.start_timestamp,
    usageRange.end_timestamp,
    'en',
    'Other',
    'Unknown'
  )
}
const imageBucket: QuotaDataItem = {
  created_at: usageRange.end_timestamp,
  model_name: 'image-only',
  count: 1,
  quota: 2000,
  token_used: null,
  token_count_scope: 'settled_text_requests',
  output_image_count: 2,
}

test('text-only token charts skip image-only buckets and preserve mixed text tokens', () => {
  const mixedBucket: QuotaDataItem = {
    ...imageBucket,
    model_name: 'mixed',
    count: 2,
    quota: 2007,
    token_used: 193,
    output_image_count: 1,
  }
  const usage = usageFor([imageBucket, mixedBucket], 'tokens')
  assert.equal(usage.total, 193)
  assert.equal(usage.values.at(-1), 193)
  assert.deepEqual(usage.models, [['mixed', 193]])
  assert.equal(usage.tokenCountScope, 'settled_text_requests')
  assert.equal(usage.hasTextTokenData, true)
  assert.equal(usage.excludedImageCount, 2)
  assert.equal(usage.outputImageCount, 3)
})

test('image-only usage retains requests and settled quota without token samples', () => {
  const tokens = usageFor([imageBucket], 'tokens')
  assert.deepEqual(tokens.models, [])
  assert.equal(tokens.tokenCountScope, 'settled_text_requests')
  assert.equal(tokens.hasTextTokenData, false)
  assert.equal(tokens.excludedImageCount, 2)
  assert.equal(tokens.outputImageCount, 2)
  assert.equal(usageFor([imageBucket], 'requests').total, 1)
  const quota = usageFor([imageBucket], 'quota')
  assert.equal(quota.total, 2000)
  assert.equal(quota.outputImageCount, 2)
  assert.equal(quota.excludedImageCount, 0)
})

test('missing or invalid token measurements remain unavailable', () => {
  const unknownBuckets: QuotaDataItem[] = [
    { ...imageBucket, token_used: undefined },
    { ...imageBucket, token_count_scope: undefined },
    { ...imageBucket, output_image_count: undefined },
    { ...imageBucket, output_image_count: null },
    { ...imageBucket, output_image_count: 0 },
    { ...imageBucket, output_image_count: 0.5 },
    { ...imageBucket, output_image_count: -1 },
    { ...imageBucket, output_image_count: Number.POSITIVE_INFINITY },
    { ...imageBucket, output_image_count: Number.MAX_SAFE_INTEGER + 1 },
    { ...imageBucket, token_used: Number.NaN },
    { ...imageBucket, token_used: -1 },
  ]
  for (const bucket of unknownBuckets) {
    assert.throws(
      () => usageFor([bucket], 'tokens'),
      /Usage metric unavailable/
    )
  }
})

test('invalid usage timestamps are unavailable even for image-only buckets', () => {
  assert.throws(
    () => usageFor([{ ...imageBucket, created_at: Number.NaN }], 'tokens'),
    /Usage timestamps unavailable/
  )
})

test('known zero text tokens remain a measurement and old image counts stay unknown', () => {
  const usage = usageFor(
    [
      {
        ...imageBucket,
        model_name: 'zero-text',
        token_used: 0,
        output_image_count: 0,
      },
    ],
    'tokens'
  )
  assert.equal(usage.total, 0)
  assert.equal(usage.hasTextTokenData, true)
  assert.deepEqual(usage.models, [['zero-text', 0]])
  assert.equal(usage.excludedImageCount, 0)
  assert.equal(usage.outputImageCount, 0)
  assert.equal(
    usageFor([{ ...imageBucket, output_image_count: undefined }], 'requests')
      .outputImageCount,
    null
  )
})

test('buckets outside the requested interval do not change image or token metadata', () => {
  const usage = usageFor(
    [{ ...imageBucket, created_at: usageRange.start_timestamp - 1 }],
    'tokens'
  )
  assert.equal(usage.tokenCountScope, undefined)
  assert.equal(usage.hasTextTokenData, false)
  assert.equal(usage.outputImageCount, 0)
  assert.equal(usage.excludedImageCount, 0)
  assert.equal(usage.total, 0)
})

test('mixed legacy buckets do not inherit native text-only scope metadata', () => {
  const usage = usageFor(
    [
      imageBucket,
      {
        created_at: usageRange.end_timestamp,
        model_name: 'legacy-text',
        token_used: 42,
      },
    ],
    'tokens'
  )
  assert.equal(usage.total, 42)
  assert.equal(usage.hasTextTokenData, true)
  assert.equal(usage.tokenCountScope, undefined)
  assert.equal(usage.outputImageCount, null)
  assert.equal(usage.excludedImageCount, 2)
})
