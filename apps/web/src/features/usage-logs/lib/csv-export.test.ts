/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { afterEach, test } from 'node:test'

import { api } from '@/lib/api'
import { useAuthStore } from '@/stores/auth-store'

import {
  MAX_USAGE_CSV_BYTES,
  readUsageCsv,
  usageCsvParams,
  UsageCsvExportError,
} from './csv-export'
import { readUsageStatisticsScope } from './statistics'

const originalGet = api.get
afterEach(() => {
  api.get = originalGet
  useAuthStore.getState().auth.reset('idle')
})
const params = { start_timestamp: 1700000000, end_timestamp: 1700000060 }
function scope(role = 1) {
  useAuthStore.getState().auth.setUser({ id: 7, username: 'csv-test', role })
  const value = readUsageStatisticsScope()
  assert.ok(value)
  return value
}
function document(isAdmin = false) {
  const data = new TextEncoder().encode(
    '\uFEFF"request_id"\r\n"test"\r\n'
  ).buffer
  return {
    data,
    headers: {
      'content-type': 'text/csv; charset=utf-8',
      'x-export-rows': '1',
      'x-export-bytes': String(data.byteLength),
      'x-export-source': isAdmin ? 'legacy_log_only' : 'native_personal_usage',
    },
  }
}
function reason(expected: string) {
  return (error: unknown) =>
    error instanceof UsageCsvExportError && error.reason === expected
}

test('export removes page limits while keeping the applied time and all supported filters', () => {
  const query = usageCsvParams({
    ...params,
    p: 9,
    page_size: 20,
    type: 0,
    model_name: 'a,b',
    token_name: 'my key',
    group: 'team',
    username: '%alice%',
    channel: 9,
    request_id: 'request',
    upstream_request_id: 'upstream',
  })
  assert.equal(query.has('p'), false)
  assert.equal(query.has('page_size'), false)
  assert.equal(query.get('type'), '0')
  assert.equal(query.get('username'), '%alice%')
  assert.equal(query.get('model_name'), 'a,b')
  assert.equal(query.get('upstream_request_id'), 'upstream')
  assert.equal(query.get('start_timestamp'), String(params.start_timestamp))
  for (const bad of [
    {},
    { start_timestamp: -1, end_timestamp: 1 },
    { start_timestamp: 2, end_timestamp: 1 },
    { start_timestamp: 0, end_timestamp: 31 * 86400 + 1 },
  ]) {
    assert.throws(() => usageCsvParams(bad), reason('invalid'))
  }
})

test('self export is a single scoped authenticated request with a bounded complete download', async () => {
  const current = scope()
  let requests = 0
  api.get = (async (url, config) => {
    requests++
    assert.match(String(url), /^\/api\/log\/self\/export\?/)
    assert.equal(String(url).includes('page_size'), false)
    assert.deepEqual(config?.authScope, { userId: 7, sessionId: undefined })
    assert.equal(config?.responseType, 'arraybuffer')
    assert.equal(config?.skipAuthRefresh, true)
    assert.equal(config?.maxContentLength, MAX_USAGE_CSV_BYTES)
    assert.equal(config?.timeout, 125000)
    return document()
  }) as typeof api.get
  const result = await readUsageCsv({
    params: { ...params, page_size: 1 },
    isAdmin: false,
    scope: current,
  })
  assert.equal(requests, 1)
  assert.equal(result.rows, 1)
  assert.equal(result.filename, 'lmm-personal-usage.csv')
  assert.match(await result.blob.text(), /"request_id"\r\n"test"/)
})

test('administrator export uses the historical source and its full filter contract', async () => {
  const current = scope(5)
  api.get = (async (url) => {
    assert.match(String(url), /^\/api\/log\/export\?/)
    const query = new URL(String(url), 'https://local.invalid').searchParams
    assert.equal(query.get('username'), 'alice')
    assert.equal(query.get('channel'), '9')
    return document(true)
  }) as typeof api.get
  assert.equal(
    (
      await readUsageCsv({
        params: { ...params, username: 'alice', channel: 9 },
        isAdmin: true,
        scope: current,
      })
    ).filename,
    'lmm-historical-usage.csv'
  )
})

test('wrong source, missing metadata, non-CSV and truncated data cannot become downloads', async () => {
  const current = scope()
  for (const changed of [
    { 'content-type': 'application/json' },
    { 'x-export-source': 'legacy_log_only' },
    { 'x-export-bytes': '1' },
    { 'x-export-rows': '10001' },
    { 'x-export-rows': '' },
  ]) {
    const valid = document()
    api.get = (async () => ({
      ...valid,
      headers: { ...valid.headers, ...changed },
    })) as typeof api.get
    await assert.rejects(
      readUsageCsv({ params, isAdmin: false, scope: current }),
      reason('incomplete')
    )
  }
  api.get = (async () => ({
    ...document(),
    data: { success: false },
  })) as typeof api.get
  await assert.rejects(
    readUsageCsv({ params, isAdmin: false, scope: current }),
    reason('incomplete')
  )
})

test('cancellation before dispatch or during a download never returns a file', async () => {
  const current = scope()
  const cancelled = new AbortController()
  cancelled.abort()
  let requests = 0
  api.get = (async () => {
    requests++
    return document()
  }) as typeof api.get
  await assert.rejects(
    readUsageCsv({
      params,
      isAdmin: false,
      scope: current,
      signal: cancelled.signal,
    }),
    reason('cancelled')
  )
  assert.equal(requests, 0)
  const controller = new AbortController()
  api.get = (async (_url, config) => {
    controller.abort()
    assert.equal(config?.signal?.aborted, true)
    return document()
  }) as typeof api.get
  await assert.rejects(
    readUsageCsv({
      params,
      isAdmin: false,
      scope: current,
      signal: controller.signal,
    }),
    reason('cancelled')
  )
})

test('account switches discard a response and reject stale requests before dispatch', async () => {
  const current = scope()
  let requests = 0
  api.get = (async () => {
    requests++
    useAuthStore
      .getState()
      .auth.setUser({ id: 8, username: 'another', role: 1 })
    return document()
  }) as typeof api.get
  await assert.rejects(
    readUsageCsv({ params, isAdmin: false, scope: current }),
    reason('session_changed')
  )
  await assert.rejects(
    readUsageCsv({ params, isAdmin: false, scope: current }),
    reason('session_changed')
  )
  assert.equal(requests, 1)
})

test('oversized transfers are aborted before a document is returned', async () => {
  const current = scope()
  api.get = (async (_url, config) => {
    config?.onDownloadProgress?.({
      loaded: MAX_USAGE_CSV_BYTES + 1,
      bytes: MAX_USAGE_CSV_BYTES + 1,
      lengthComputable: false,
    })
    assert.equal(config?.signal?.aborted, true)
    return document()
  }) as typeof api.get
  await assert.rejects(
    readUsageCsv({ params, isAdmin: false, scope: current }),
    reason('too_dense')
  )
})

test('server density, concurrency, permission and timeout errors retain actionable states without retries', async () => {
  const current = scope()
  for (const [status, expected] of [
    [429, 'busy'],
    [403, 'forbidden'],
    [504, 'timeout'],
    [503, 'unavailable'],
    [400, 'too_dense'],
  ] as const) {
    api.get = (async () => {
      throw Object.assign(new Error('server refusal'), {
        isAxiosError: true,
        response: {
          status,
          data: new TextEncoder().encode(
            JSON.stringify({ code: 'USAGE_EXPORT_TOO_DENSE' })
          ).buffer,
        },
      })
    }) as typeof api.get
    await assert.rejects(
      readUsageCsv({ params, isAdmin: false, scope: current }),
      reason(expected)
    )
  }
})
