/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import type { LogCategory } from '../types'
import {
  parseHistoryUnavailable,
  parseLogsPage,
  UsageHistoryUnavailableError,
} from './history-response'

const nativeLog = {
  id: 81,
  user_id: 7,
  created_at: 1700000000,
  type: 2,
  content: 'synthetic native usage',
  quota: 17,
  prompt_tokens: 3,
  completion_tokens: 4,
}
const historicalLog = {
  source_log_id: 701,
  user_id: 7,
  token_id: null,
  created_at: 1700000000,
  type: 2,
  username: 'synthetic-user',
  token_name: 'retired key',
  model_name: 'synthetic-model',
  quota: null,
  prompt_tokens: null,
  completion_tokens: null,
  group: 'default',
  request_id: 'synthetic-historical-request',
  quota_money_available: false,
  quota_money_basis: 'legacy_unit_unknown',
}
const drawingLog = {
  id: 21,
  user_id: 7,
  channel_id: 4,
  code: 1,
  mj_id: 'synthetic-mj-id',
  action: 'IMAGINE',
  submit_time: 1700000000000,
  progress: '100%',
  prompt: 'synthetic drawing',
  status: 'SUCCESS',
}
const taskLog = {
  id: 31,
  user_id: 7,
  channel_id: 4,
  task_id: 'synthetic-task-id',
  platform: 'synthetic-platform',
  action: 'GENERATE',
  submit_time: 1700000000,
  status: 'SUCCESS',
  data: { synthetic: true },
}
const categories: readonly LogCategory[] = ['common', 'drawing', 'task']
const rows: Record<LogCategory, unknown> = {
  common: nativeLog,
  drawing: drawingLog,
  task: taskLog,
}

function page(items: unknown[] = []) {
  return { total: items.length, page: 1, page_size: 20, items }
}

test('native usage pages retain their own kind and validated schema defaults', () => {
  const result = parseLogsPage(
    { success: true, data: page([nativeLog]) },
    'common'
  )
  assert.equal(result.kind, 'common')
  if (result.kind !== 'common') assert.fail('expected native usage page')
  assert.equal(result.items[0].id, 81)
  assert.equal(result.items[0].quota, 17)
  assert.equal(result.items[0].prompt_tokens, 3)
  assert.equal(result.items[0].completion_tokens, 4)
  assert.equal(result.items[0].token_id, null)
})

test('legacy log pages preserve source identity and unknown values without native defaults', () => {
  const data = { ...page([historicalLog]), source: 'legacy_log_only' }
  const result = parseLogsPage({ success: true, data }, 'common')
  assert.equal(result.kind, 'historical')
  assert.deepEqual(result, { ...data, kind: 'historical' })
  assert.equal(Object.hasOwn(result.items[0], 'id'), false)
  assert.equal(Object.hasOwn(result.items[0], 'content'), false)
  assert.equal(Object.hasOwn(result.items[0], 'quota_usd'), false)
})

test('native OAuth receipts retain an explicit null API key identity', () => {
  const result = parseLogsPage(
    { success: true, data: page([{ ...nativeLog, token_id: null }]) },
    'common'
  )
  assert.equal(result.kind, 'common')
  if (result.kind !== 'common') assert.fail('expected native usage page')
  assert.equal(result.items[0].token_id, null)
  assert.equal(result.items[0].id, nativeLog.id)
})

test('historical signed counters and captured money strings remain exact', () => {
  const row = {
    ...historicalLog,
    token_id: 9,
    quota: -17,
    prompt_tokens: 0,
    completion_tokens: 4,
    quota_usd: '0.000000000000000017',
    quota_money_available: true,
    quota_money_basis: 'captured_credits_per_usd',
  }
  const result = parseLogsPage(
    { success: true, data: { ...page([row]), source: 'legacy_log_only' } },
    'common'
  )
  assert.equal(result.kind, 'historical')
  assert.deepEqual(result.items, [row])
})

test('each log category accepts a proved empty page and its own nonempty row', () => {
  for (const category of categories) {
    assert.deepEqual(parseLogsPage({ success: true, data: page() }, category), {
      ...page(),
      kind: category,
    })
    assert.equal(
      parseLogsPage({ success: true, data: page([rows[category]]) }, category)
        .kind,
      category
    )
  }
})

test('missing and failed envelopes never become empty history', () => {
  const invalid: unknown[] = [
    null,
    [],
    {},
    { success: false, data: page() },
    { success: 'true', data: page() },
    { success: true },
    { success: true, data: null },
    { success: true, data: [] },
    { success: true, data: {} },
  ]
  for (const category of categories) {
    for (const response of invalid) {
      assert.throws(
        () => parseLogsPage(response, category),
        /history response unavailable/
      )
    }
  }
})

test('pagination and items must be explicitly present and valid', () => {
  const good = page()
  const invalid: unknown[] = [
    { ...good, items: null },
    { ...good, items: {} },
  ]
  for (const key of ['page', 'page_size', 'total', 'items']) {
    const missing: Record<string, unknown> = { ...good }
    delete missing[key]
    invalid.push(missing)
  }
  for (const key of ['page', 'page_size', 'total']) {
    for (const value of [
      null,
      '1',
      -1,
      0.5,
      Infinity,
      Number.NaN,
      Number.MAX_SAFE_INTEGER + 1,
    ]) {
      invalid.push({ ...good, [key]: value })
    }
  }
  invalid.push({ ...good, page: 0 }, { ...good, page_size: 0 })
  for (const category of categories) {
    for (const data of invalid) {
      assert.throws(
        () => parseLogsPage({ success: true, data }, category),
        /history response unavailable/
      )
    }
  }
})

test('drawing and task pages cannot silently consume common usage or each other', () => {
  for (const [category, row] of [
    ['drawing', nativeLog],
    ['drawing', taskLog],
    ['task', nativeLog],
    ['task', drawingLog],
    ['common', drawingLog],
    ['common', taskLog],
  ] satisfies Array<[LogCategory, unknown]>) {
    assert.throws(
      () => parseLogsPage({ success: true, data: page([row]) }, category),
      /history response unavailable/
    )
  }
})

test('a legacy source marker requires the full historical row instead of falling back to native usage', () => {
  const invalidRows: unknown[] = [
    nativeLog,
    null,
    { ...historicalLog, quota: '0' },
  ]
  for (const key of [
    'source_log_id',
    'token_id',
    'quota',
    'prompt_tokens',
    'completion_tokens',
    'quota_money_available',
    'quota_money_basis',
  ]) {
    const missing: Record<string, unknown> = { ...historicalLog }
    delete missing[key]
    invalidRows.push(missing)
  }
  invalidRows.push({
    ...historicalLog,
    source_log_id: Number.MAX_SAFE_INTEGER + 1,
  })
  for (const row of invalidRows) {
    assert.throws(
      () =>
        parseLogsPage(
          {
            success: true,
            data: { ...page([row]), source: 'legacy_log_only' },
          },
          'common'
        ),
      /history response unavailable/
    )
  }
  assert.throws(
    () =>
      parseLogsPage({ success: true, data: page([historicalLog]) }, 'common'),
    /history response unavailable/
  )
})

test('unavailable history requires the exact failure code, state and requested category', () => {
  for (const category of ['drawing', 'task'] satisfies LogCategory[]) {
    const payload = {
      success: false,
      code: 'USAGE_HISTORY_UNAVAILABLE',
      availability: 'unavailable',
      category,
      message: 'Synthetic category unavailable',
    }
    const result = parseHistoryUnavailable(payload, category)
    assert.ok(result instanceof UsageHistoryUnavailableError)
    assert.equal(result.category, category)
    assert.equal(result.name, 'UsageHistoryUnavailableError')
    assert.equal(parseHistoryUnavailable(payload, 'common'), undefined)
    assert.equal(
      parseHistoryUnavailable(
        payload,
        category === 'task' ? 'drawing' : 'task'
      ),
      undefined
    )
    for (const invalid of [
      null,
      {},
      { ...payload, success: true },
      { ...payload, code: 'SERVICE_UNAVAILABLE' },
      { ...payload, availability: 'available' },
      { ...payload, category: 'common' },
    ]) {
      assert.equal(parseHistoryUnavailable(invalid, category), undefined)
    }
    for (const key of ['success', 'code', 'availability', 'category']) {
      const missing: Record<string, unknown> = { ...payload }
      delete missing[key]
      assert.equal(parseHistoryUnavailable(missing, category), undefined)
    }
  }
})
