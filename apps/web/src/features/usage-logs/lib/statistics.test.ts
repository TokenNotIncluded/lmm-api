/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { afterEach, test } from 'node:test'

import { api } from '@/lib/api'
import { useAuthStore } from '@/stores/auth-store'

import {
  parseUsageStatistics,
  readUsageStatistics,
  readUsageStatisticsScope,
} from './statistics'

const originalGet = api.get
afterEach(() => {
  api.get = originalGet
  useAuthStore.getState().auth.reset('idle')
})

test('accepts exact native counters and the server-owned USD string', () => {
  const data = {
    quota: 500000,
    rpm: 3,
    tpm: 9,
    quota_usd: '1.000000000000000000',
    quota_money_available: true,
    quota_money_basis: 'captured_credits_per_usd',
  }
  assert.deepEqual(parseUsageStatistics({ success: true, data }), data)
})

test('preserves real zero and legacy raw counters without fabricating money metadata', () => {
  const data = { quota: 0, rpm: 0, tpm: 0 }
  assert.deepEqual(parseUsageStatistics({ success: true, data }), data)
  assert.equal(
    Object.hasOwn(parseUsageStatistics({ success: true, data }), 'quota_usd'),
    false
  )
})

function legacyStatistics() {
  return {
    quota: 193,
    raw_consumption_quota: '193',
    quota_numeric_representable: true,
    invalid_quota_records: 1,
    unpriced_quota_records: 2,
    quota_projection_truncated: false,
    quota_money_available: false,
    quota_money_basis: 'aggregate_missing_credit_basis',
    quota_usd: null,
    rpm: null,
    tpm: null,
    rates_available: false,
    source: 'legacy_log_only',
  }
}

test('accepts the real Core historical DTO and preserves unavailable rates and money', () => {
  const data = legacyStatistics()
  assert.deepEqual(parseUsageStatistics({ success: true, data }), data)
})

test('preserves exact historical quota strings beyond the JavaScript safe integer range', () => {
  const data = {
    ...legacyStatistics(),
    quota: null,
    raw_consumption_quota: '900719925474099312345678901234567890',
    quota_numeric_representable: false,
  }
  assert.deepEqual(parseUsageStatistics({ success: true, data }), data)
})

test('an invalid historical raw string remains an unknown source fact rather than acquiring zero', () => {
  for (const raw of ['', 'not-an-integer', '193.5', '1e3']) {
    const data = {
      ...legacyStatistics(),
      quota: null,
      raw_consumption_quota: raw,
      quota_numeric_representable: false,
    }
    assert.deepEqual(parseUsageStatistics({ success: true, data }), data)
  }
})

test('an explicitly unavailable numeric quota does not discard its exact raw string', () => {
  const data = {
    ...legacyStatistics(),
    quota: null,
    quota_numeric_representable: false,
  }
  assert.deepEqual(parseUsageStatistics({ success: true, data }), data)
})

test('nullable historical fields require the explicit Core legacy contract', () => {
  const data = legacyStatistics()
  const invalid: unknown[] = [
    { ...data, source: undefined },
    { ...data, source: 'native' },
    { ...data, raw_consumption_quota: undefined },
    { ...data, raw_consumption_quota: 193 },
    { ...data, raw_consumption_quota: null },
    { ...data, quota_numeric_representable: undefined },
    { ...data, quota_numeric_representable: 'true' },
    { ...data, quota_numeric_representable: false },
    { ...data, quota: null },
    { ...data, quota: Number.MAX_SAFE_INTEGER + 1 },
    { ...data, invalid_quota_records: undefined },
    { ...data, invalid_quota_records: -1 },
    { ...data, unpriced_quota_records: undefined },
    { ...data, quota_usd: undefined },
    { ...data, quota_money_available: undefined },
    { ...data, quota_money_basis: undefined },
    { ...data, rates_available: undefined },
    { ...data, rates_available: true },
    { ...data, rpm: 0 },
    { ...data, tpm: 0 },
  ]
  for (const value of invalid) {
    assert.throws(
      () => parseUsageStatistics({ success: true, data: value }),
      /statistics unavailable/
    )
  }
})

test('invalid legacy markers cannot escape validation through native numeric fields', () => {
  const data = { ...legacyStatistics(), rpm: 0, tpm: 0 }
  for (const source of ['native', undefined]) {
    assert.throws(
      () => parseUsageStatistics({ success: true, data: { ...data, source } }),
      /statistics unavailable/
    )
  }
})

test('contradictory complete money projections become unknown without discarding valid counters', () => {
  for (const data of [
    {
      quota: 817,
      rpm: 29,
      tpm: 743,
      quota_usd: '0',
      quota_money_available: true,
      quota_money_basis: 'empty_result',
    },
    {
      quota: 0,
      rpm: 29,
      tpm: 743,
      quota_usd: '7.250000',
      quota_money_available: true,
      quota_money_basis: 'captured_credits_per_usd',
    },
    {
      quota: 817,
      rpm: 29,
      tpm: 743,
      quota_usd: '0',
      quota_money_available: true,
      quota_money_basis: 'captured_credits_per_usd',
    },
  ]) {
    const parsed = parseUsageStatistics({ success: true, data })
    assert.equal(parsed.quota, data.quota)
    assert.equal(parsed.rpm, 29)
    assert.equal(parsed.tpm, 743)
    assert.equal(parsed.quota_money_available, false)
    assert.equal(Object.hasOwn(parsed, 'quota_usd'), false)
  }
})

test('failed, missing or invalid statistics never acquire zero defaults', () => {
  const good = { quota: 1, rpm: 2, tpm: 3 }
  const invalid: unknown[] = [
    null,
    {},
    { success: false, data: good },
    { success: true },
    { success: true, data: null },
    { success: true, data: { raw_consumption_quota: '100', rpm: 2, tpm: 3 } },
  ]
  for (const key of ['quota', 'rpm', 'tpm'] as const) {
    const missing: Partial<typeof good> = { ...good }
    delete missing[key]
    invalid.push({ success: true, data: missing })
    for (const value of [
      null,
      '0',
      -1,
      0.5,
      Infinity,
      Number.NaN,
      Number.MAX_SAFE_INTEGER + 1,
    ]) {
      invalid.push({ success: true, data: { ...good, [key]: value } })
    }
  }
  for (const response of invalid) {
    assert.throws(
      () => parseUsageStatistics(response),
      /statistics unavailable/
    )
  }
})

test('scoped reads use the existing self endpoint and do not refresh or retry authentication', async () => {
  useAuthStore
    .getState()
    .auth.setUser({ id: 7, username: 'synthetic', role: 1 })
  const scope = readUsageStatisticsScope()
  assert.ok(scope)
  let reads = 0
  api.get = (async (url: string, config: Record<string, unknown>) => {
    reads += 1
    assert.match(url, /^\/api\/log\/self\/stat\?/)
    assert.deepEqual(config.authScope, { userId: 7, sessionId: undefined })
    assert.equal(config.skipAuthRefresh, true)
    assert.equal(config.timeout, 10_000)
    return { data: { success: true, data: { quota: 0, rpm: 0, tpm: 0 } } }
  }) as typeof api.get
  assert.deepEqual(await readUsageStatistics(scope, {}, false), {
    quota: 0,
    rpm: 0,
    tpm: 0,
  })
  assert.equal(reads, 1)
})

test('account switches reject old requests before dispatch and after their response', async () => {
  useAuthStore.getState().auth.setUser({ id: 7, username: 'first', role: 1 })
  const scope = readUsageStatisticsScope()
  assert.ok(scope)
  let reads = 0
  api.get = (async () => {
    reads += 1
    useAuthStore.getState().auth.setUser({ id: 8, username: 'second', role: 1 })
    return { data: { success: true, data: { quota: 999, rpm: 99, tpm: 99 } } }
  }) as typeof api.get
  await assert.rejects(readUsageStatistics(scope, {}, false), /session changed/)
  await assert.rejects(readUsageStatistics(scope, {}, false), /session changed/)
  assert.equal(reads, 1)
})
