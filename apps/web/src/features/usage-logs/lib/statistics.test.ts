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
