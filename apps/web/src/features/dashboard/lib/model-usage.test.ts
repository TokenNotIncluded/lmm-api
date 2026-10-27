/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.
*/
import assert from 'node:assert/strict'
import { test } from 'node:test'

import type { QuotaDataItem } from '../types'
import {
  modelUsageCsv,
  selectModelUsage,
  summarizeModelUsage,
} from './model-usage'

const descending = { metric: 'quota', direction: 'descending' } as const
const data = [
  { created_at: 1, model_name: 'alpha', count: 2, token_used: 20, quota: 3 },
  { created_at: 2, model_name: 'alpha', count: 1, token_used: 10, quota: 1 },
  { created_at: 1, model_name: 'beta', count: 4, token_used: 80, quota: 6 },
]

const nativeBucket = (
  overrides: Partial<QuotaDataItem> = {}
): QuotaDataItem => ({
  created_at: 0,
  model_name: 'image',
  count: 1,
  quota: 100,
  token_used: null,
  output_image_count: 2,
  token_count_scope: 'settled_text_requests',
  usage_scope: 'actor_personal_usage',
  settlement_scope: 'settled_requests',
  platform_sponsored: false,
  wallet_debit_supported: false,
  ...overrides,
})

test('native image-only models retain null tokens and exact image/consumption counts', () => {
  const [row] = summarizeModelUsage([nativeBucket()])
  assert.equal(row.tokens, null)
  assert.equal(row.count, 1)
  assert.equal(row.quota_raw, '100')
  assert.equal(row.output_image_count, 2)
  assert.equal(row.token_count_scope, 'settled_text_requests')
  assert.equal(row.settlement_scope, 'settled_requests')
  assert.equal(row.wallet_debit_supported, false)
  const record = modelUsageCsv([row], [], '').split('\r\n')[1]
  assert.match(record, /^"image","1","","100"/)
  assert.ok(record.endsWith(',"2","settled_text_requests"'))
})

test('model aggregation retains administrator scope instead of relabeling personal usage', () => {
  const [row] = summarizeModelUsage([
    nativeBucket({ usage_scope: 'administrator_all_accounts' }),
  ])
  assert.equal(row.usage_scope, 'administrator_all_accounts')
  assert.equal(row.quota_raw, '100')
  assert.equal(row.tokens, null)
})

test('a model with text and images keeps known text tokens and separates sponsored consumption', () => {
  const [row] = summarizeModelUsage([
    nativeBucket(),
    nativeBucket({
      token_used: 19,
      output_image_count: 0,
      quota: 25,
      platform_sponsored: true,
    }),
  ])
  assert.equal(row.tokens, 19)
  assert.equal(row.output_image_count, 2)
  assert.equal(row.count, 2)
  assert.equal(row.quota_raw, '125')
  assert.equal(row.platform_sponsored_quota, 25)
  assert.equal(row.token_count_scope, 'settled_text_requests')
})

test('unknown legacy tokens do not become zero or a known subtotal, while real zero remains known', () => {
  const unknown = summarizeModelUsage([
    {
      created_at: 0,
      model_name: 'unknown',
      token_used: null,
      count: 1,
      quota: 3,
    },
    {
      created_at: 0,
      model_name: 'unknown',
      token_used: 19,
      count: 1,
      quota: 2,
    },
  ])[0]
  assert.equal(unknown.tokens, null)
  assert.equal(unknown.count, 2)
  assert.equal(unknown.quota_raw, '5')
  const zero = summarizeModelUsage([
    nativeBucket({ model_name: 'zero', token_used: 0, output_image_count: 0 }),
  ])[0]
  assert.equal(zero.tokens, 0)
  for (const direction of ['ascending', 'descending'] as const) {
    assert.equal(
      selectModelUsage(
        [unknown, zero],
        '',
        { metric: 'tokens', direction },
        ''
      )[0].model,
      'zero'
    )
  }
})

test('mixing native and legacy buckets leaves incomplete image totals and scope unavailable', () => {
  const [row] = summarizeModelUsage([
    nativeBucket(),
    { created_at: 1, model_name: 'image', token_used: 5, quota: 10 },
  ])
  assert.equal(row.tokens, 5)
  assert.equal(row.output_image_count, null)
  assert.equal(row.token_count_scope, undefined)
  assert.equal(row.platform_sponsored_quota, null)
})

test('combines time buckets and users without changing the input', () => {
  const input = structuredClone(data)
  assert.deepEqual(summarizeModelUsage(input), [
    {
      model: 'alpha',
      count: 3,
      tokens: 30,
      quota: 4,
      quota_raw: '4',
      share: 0.4,
    },
    {
      model: 'beta',
      count: 4,
      tokens: 80,
      quota: 6,
      quota_raw: '6',
      share: 0.6,
    },
  ])
  assert.deepEqual(input, data)
})

test('search does not change the share denominator or original order', () => {
  const rows = summarizeModelUsage(data)
  const selected = selectModelUsage(rows, '  ALPHA  ', descending, 'Unknown')
  assert.equal(selected.length, 1)
  assert.equal(selected[0].share, 0.4)
  assert.equal(rows[0].model, 'alpha')
  assert.equal(
    selectModelUsage(rows, '', descending, 'Unknown')[0].model,
    'beta'
  )
})

test('supports every numeric sort direction and deterministic ties', () => {
  const rows = summarizeModelUsage(data)
  for (const metric of ['count', 'tokens', 'quota'] as const) {
    assert.equal(
      selectModelUsage(rows, '', { metric, direction: 'ascending' }, '')[0]
        .model,
      'alpha'
    )
    assert.equal(
      selectModelUsage(rows, '', { metric, direction: 'descending' }, '')[0]
        .model,
      'beta'
    )
  }
  const ties = summarizeModelUsage([
    { created_at: 0, model_name: 'z', quota: 1 },
    { created_at: 0, model_name: 'a', quota: 1 },
  ])
  assert.deepEqual(
    selectModelUsage(ties, '', descending, '').map((r) => r.model),
    ['a', 'z']
  )
})

test('empty data and zero cost do not invent a percentage', () => {
  assert.deepEqual(summarizeModelUsage([]), [])
  const rows = summarizeModelUsage([
    { created_at: 0, model_name: 'free', count: 3, quota: 0 },
  ])
  assert.deepEqual(rows, [
    {
      model: 'free',
      count: 3,
      tokens: null,
      quota: 0,
      quota_raw: '0',
      share: null,
    },
  ])
  assert.deepEqual(selectModelUsage(rows, 'not-found', descending, ''), [])
})

test('keeps unknown, literal Unknown, and object-like names separate', () => {
  const rows = summarizeModelUsage(
    ['', 'Unknown', '__proto__', 'constructor'].map((model_name) => ({
      created_at: 0,
      model_name,
      quota: 1,
    }))
  )
  assert.equal(rows.length, 4)
  assert.equal(
    selectModelUsage(rows, '未知', descending, '未知模型')[0].model,
    ''
  )
})

test('does not leak invalid numbers and preserves signed adjustments', () => {
  const rows = summarizeModelUsage([
    {
      created_at: 0,
      count: Number.NaN,
      token_used: Number.POSITIVE_INFINITY,
      quota: -2,
    },
    { created_at: 1, quota: 5 },
  ])
  assert.deepEqual(rows, [
    { model: '', count: 0, tokens: null, quota: 3, quota_raw: '3', share: 1 },
  ])
})

test('CSV preserves raw quota and declares the missing historical money basis', () => {
  const csv = modelUsageCsv(
    summarizeModelUsage(data),
    [
      'Model',
      'Calls',
      'Tokens',
      'Raw quota',
      'Quota unit',
      'Historical money available',
      'Historical money basis',
      'Quota share (%)',
    ],
    'Unknown'
  )
  assert.ok(csv.startsWith('\uFEFF"Model"'))
  assert.ok(
    csv.includes(
      '"alpha","3","30","4","raw_quota","false","bucket_without_credit_basis","40"'
    )
  )
  assert.doesNotMatch(csv, /USD|CNY/)
  assert.ok(csv.endsWith('\r\n'))
})

test('unverified bucket metadata cannot turn raw quota into a historical amount', () => {
  const rows = summarizeModelUsage([
    {
      created_at: 0,
      model_name: 'legacy',
      quota: 3_500_000,
      quota_usd: '7',
      quota_money_available: true,
      quota_money_basis: 'current_wallet_fx',
    },
  ])
  assert.equal(
    modelUsageCsv(rows, [], '').split('\r\n')[1],
    '"legacy","0","","3500000","raw_quota","false","bucket_without_credit_basis","100"'
  )
})

test('raw quota sums remain exact after crossing the safe-number boundary', () => {
  const rows = summarizeModelUsage([
    { created_at: 0, model_name: 'large', quota: Number.MAX_SAFE_INTEGER },
    { created_at: 1, model_name: 'large', quota: 2 },
  ])
  assert.equal(rows[0].quota_raw, '9007199254740993')
  assert.equal(
    modelUsageCsv(rows, [], '').split('\r\n')[1],
    '"large","0","","9007199254740993","raw_quota","false","bucket_without_credit_basis","100"'
  )
})

test('an inexact source leaves raw quota unknown without discarding usage counts', () => {
  for (const quota of [
    Number.MAX_SAFE_INTEGER + 1,
    0.5,
    Number.NaN,
    Number.POSITIVE_INFINITY,
    null as unknown as number,
    undefined,
  ]) {
    const rows = summarizeModelUsage([
      { created_at: 0, model_name: 'invalid', count: 2, token_used: 20, quota },
      { created_at: 1, model_name: 'invalid', quota: 1 },
    ])
    assert.equal(rows[0].quota_raw, null)
    assert.equal(rows[0].count, 2)
    assert.equal(rows[0].tokens, null)
    assert.equal(rows[0].share, null)
    assert.match(
      modelUsageCsv(rows, [], '').split('\r\n')[1],
      /^"invalid","2","","","raw_quota","false","raw_quota_not_exact",/
    )
  }
  const signed = summarizeModelUsage([
    { created_at: 0, model_name: 'adjustment', quota: -2 },
  ])
  assert.equal(signed[0].quota_raw, '-2')
  assert.equal(
    modelUsageCsv(signed, [], '').split('\r\n')[1],
    '"adjustment","0","","-2","raw_quota","false","bucket_without_credit_basis",""'
  )
})

test('any unknown raw amount leaves every period share unknown, while real zero stays known', () => {
  for (const quota of [
    Number.MAX_SAFE_INTEGER + 1,
    0.5,
    Number.NaN,
    Number.POSITIVE_INFINITY,
    null as unknown as number,
    undefined,
  ]) {
    const rows = summarizeModelUsage([
      { created_at: 0, model_name: 'unknown', quota },
      { created_at: 0, model_name: 'known', quota: 1 },
    ])
    assert.equal(rows[0].quota_raw, null)
    assert.equal(rows[1].quota_raw, '1')
    assert.deepEqual(
      rows.map((row) => row.share),
      [null, null]
    )
    const exported = modelUsageCsv(rows, [], '').split('\r\n')
    assert.ok(exported[1].endsWith(',""'))
    assert.ok(exported[2].endsWith(',""'))
  }
  const rows = summarizeModelUsage([
    { created_at: 0, model_name: 'zero', quota: 0 },
    { created_at: 0, model_name: 'known', quota: 1 },
  ])
  assert.deepEqual(
    rows.map((row) => row.quota_raw),
    ['0', '1']
  )
  assert.deepEqual(
    rows.map((row) => row.share),
    [0, 1]
  )
  const invalidStored = summarizeModelUsage([
    {
      created_at: 0,
      model_name: 'null-sentinel',
      quota: 0,
      quota_money_available: false,
      quota_money_basis: 'invalid_stored_quota',
    },
    { created_at: 0, model_name: 'known', quota: 1 },
  ])
  assert.deepEqual(
    invalidStored.map((row) => row.quota_raw),
    [null, '1']
  )
  assert.deepEqual(
    invalidStored.map((row) => row.share),
    [null, null]
  )
})

test('CSV escapes quotes, line breaks, and formula-like model names', () => {
  for (const model of [
    '=1+1',
    '+1',
    '-cmd',
    '@SUM(1)',
    ' \t=1',
    '\r=1',
    '\uFEFF=1',
  ]) {
    const rows = summarizeModelUsage([{ created_at: 0, model_name: model }])
    assert.ok(modelUsageCsv(rows, [], '').includes(`"'${model}"`))
  }
  const rows = summarizeModelUsage([{ created_at: 0, model_name: 'a,"b\nc' }])
  assert.ok(modelUsageCsv(rows, [], '').includes('"a,""b\nc"'))
})

test('CSV includes the whole searched set, not only a visible page', () => {
  const rows = summarizeModelUsage(
    Array.from({ length: 35 }, (_, i) => ({
      created_at: 0,
      model_name: `model-${i}`,
      quota: i,
    }))
  )
  const selected = selectModelUsage(rows, 'model-', descending, '')
  assert.equal(modelUsageCsv(selected, ['Model'], '').split('\r\n').length, 37)
})

test('feature translations cover all locales with matching placeholders', async () => {
  const { dashboardUsageTranslations } = await import('../model-usage-i18n')
  const english = dashboardUsageTranslations.en
  const keys = Object.keys(english).sort()
  const placeholders = (text: string) =>
    [...text.matchAll(/{{(\w+)}}/g)].map((m) => m[1]).sort()
  assert.deepEqual(Object.keys(dashboardUsageTranslations).sort(), [
    'en',
    'fr',
    'ja',
    'ru',
    'vi',
    'zhCN',
    'zhTW',
  ])
  for (const resources of Object.values(dashboardUsageTranslations)) {
    assert.deepEqual(Object.keys(resources).sort(), keys)
    for (const [key, value] of Object.entries(resources)) {
      assert.ok(value.trim())
      assert.deepEqual(
        placeholders(value),
        placeholders(english[key as keyof typeof english])
      )
    }
  }
})
