/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { afterEach, test } from 'node:test'

import { AxiosError, type InternalAxiosRequestConfig } from 'axios'
import { toast } from 'sonner'

import { api } from '@/lib/api'
import { useAuthStore } from '@/stores/auth-store'

import {
  getAllLogs,
  getAllMidjourneyLogs,
  getAllTaskLogs,
  getUserLogs,
  getUserMidjourneyLogs,
  getUserTaskLogs,
  readLogsPage,
} from './api'
import { UsageHistoryUnavailableError } from './lib/history-response'
import { readUsageStatisticsScope } from './lib/statistics'
import type { LogCategory } from './types'

const originalAdapter = api.defaults.adapter
const originalToastError = toast.error
const previousAuth = useAuthStore.getState().auth

function login({ id = 7, sid = 'synthetic-history-session', role = 1 } = {}) {
  useAuthStore.getState().auth.setBundle({
    access_token: 'synthetic-history-credential',
    token_type: 'Bearer',
    access_expires_at: 2000000000,
    user: { id, username: 'synthetic-history-user', role },
    session: {
      sid,
      current: true,
      login_method: 'password',
      ip: '127.0.0.1',
      user_agent: 'unit-only',
      created_at: 1,
      last_active_at: 1,
      expires_at: 2000000000,
    },
  })
}

function response(
  config: InternalAxiosRequestConfig,
  data: unknown,
  status = 200
) {
  return { config, data, status, statusText: 'Synthetic', headers: {} }
}

function emptyPage() {
  return {
    success: true,
    data: { page: 1, page_size: 20, total: 0, items: [] },
  }
}

function unavailable(category: 'drawing' | 'task') {
  return {
    success: false,
    code: 'USAGE_HISTORY_UNAVAILABLE',
    availability: 'unavailable',
    category,
    message: 'Synthetic category unavailable',
  }
}

afterEach(() => {
  api.defaults.adapter = originalAdapter
  toast.error = originalToastError
  useAuthStore.setState({ auth: previousAuth })
})

test('history uses the exact category and self/admin endpoints with scoped, bounded reads', async () => {
  const endpoints: Record<LogCategory, string> = {
    common: '/api/log',
    drawing: '/api/mj',
    task: '/api/task',
  }
  const seen: string[] = []
  const controller = new AbortController()
  for (const category of [
    'common',
    'drawing',
    'task',
  ] satisfies LogCategory[]) {
    for (const isAdmin of [false, true]) {
      login({ role: isAdmin ? 10 : 1 })
      api.defaults.adapter = async (config) => {
        seen.push(config.url ?? '')
        assert.equal(config.method, 'get')
        assert.deepEqual(config.authScope, {
          userId: 7,
          sessionId: 'synthetic-history-session',
        })
        assert.equal(config.skipAuthRefresh, true)
        assert.equal(config.skipBusinessError, true)
        assert.equal(config.skipErrorHandler, true)
        assert.equal(config.disableDuplicate, true)
        assert.equal(config.timeout, 10_000)
        assert.equal(config.signal, controller.signal)
        return response(config, emptyPage())
      }
      assert.equal(
        (
          await readLogsPage({
            category,
            params: {},
            isAdmin,
            signal: controller.signal,
          })
        ).kind,
        category
      )
    }
  }
  assert.deepEqual(
    seen,
    Object.values(endpoints).flatMap((endpoint) => [
      `${endpoint}/self?p=1&page_size=20`,
      `${endpoint}?p=1&page_size=20`,
    ])
  )
})

test('legacy exported readers all flow through the typed category boundary', async () => {
  login({ role: 10 })
  const seen: string[] = []
  api.defaults.adapter = async (config) => {
    seen.push(config.url ?? '')
    return response(config, emptyPage())
  }
  const readers = [
    () => getAllLogs(),
    () => getUserLogs(),
    () => getAllMidjourneyLogs({}),
    () => getUserMidjourneyLogs({}),
    () => getAllTaskLogs({}),
    () => getUserTaskLogs({}),
  ]
  const kinds = ['common', 'common', 'drawing', 'drawing', 'task', 'task']
  for (const [index, read] of readers.entries()) {
    const result = await read()
    assert.equal(result.success, true)
    assert.equal(result.data?.kind, kinds[index])
  }
  assert.deepEqual(seen, [
    '/api/log?p=1&page_size=20',
    '/api/log/self?p=1&page_size=20',
    '/api/mj?p=1&page_size=20',
    '/api/mj/self?p=1&page_size=20',
    '/api/task?p=1&page_size=20',
    '/api/task/self?p=1&page_size=20',
  ])
})

test('history query parameters retain explicit zeroes and encode literal filters', async () => {
  login()
  let reads = 0
  api.defaults.adapter = async (config) => {
    reads += 1
    const url = new URL(config.url ?? '', 'https://synthetic.example.invalid')
    assert.equal(url.pathname, '/api/log/self')
    assert.deepEqual(Object.fromEntries(url.searchParams), {
      p: '3',
      page_size: '50',
      type: '0',
      token_name: ' key & +%_ ',
      channel: '0',
      start_timestamp: '0',
    })
    return response(config, emptyPage())
  }
  await readLogsPage({
    category: 'common',
    isAdmin: false,
    params: {
      p: 3,
      page_size: 50,
      type: 0,
      token_name: ' key & +%_ ',
      channel: 0,
      start_timestamp: 0,
      group: '',
      request_id: undefined,
    },
  })
  assert.equal(reads, 1)
})

test('the matching structured 503 becomes explicit unavailable rather than successful empty history', async () => {
  for (const category of ['drawing', 'task'] satisfies Array<
    'drawing' | 'task'
  >) {
    login()
    let reads = 0
    let toasts = 0
    toast.error = () => {
      toasts += 1
      return 'synthetic-history-toast'
    }
    api.defaults.adapter = async (config) => {
      reads += 1
      throw new AxiosError(
        'Synthetic unavailable',
        'ERR_BAD_RESPONSE',
        config,
        undefined,
        response(config, unavailable(category), 503)
      )
    }
    await assert.rejects(
      readLogsPage({ category, params: {}, isAdmin: false }),
      (error: unknown) =>
        error instanceof UsageHistoryUnavailableError &&
        error.category === category
    )
    assert.equal(reads, 1)
    assert.equal(toasts, 0)
    assert.equal(useAuthStore.getState().auth.user?.id, 7)
  }
})

test('401, 403, ordinary 503 and category mismatches remain failures without retry or toast', async () => {
  const cases = [
    { status: 401, category: 'drawing', data: unavailable('drawing') },
    { status: 403, category: 'drawing', data: unavailable('drawing') },
    {
      status: 503,
      category: 'drawing',
      data: { success: false, message: 'Busy' },
    },
    { status: 503, category: 'drawing', data: unavailable('task') },
    { status: 503, category: 'common', data: unavailable('drawing') },
    {
      status: 503,
      category: 'drawing',
      data: { ...unavailable('drawing'), availability: 'available' },
    },
  ] satisfies Array<{ status: number; category: LogCategory; data: unknown }>
  for (const entry of cases) {
    login()
    let reads = 0
    let toasts = 0
    let failure: AxiosError | undefined
    toast.error = () => {
      toasts += 1
      return 'synthetic-history-toast'
    }
    api.defaults.adapter = async (config) => {
      reads += 1
      failure = new AxiosError(
        'Synthetic failure',
        'ERR_BAD_RESPONSE',
        config,
        undefined,
        response(config, entry.data, entry.status)
      )
      throw failure
    }
    await assert.rejects(
      readLogsPage({ category: entry.category, params: {}, isAdmin: false }),
      (error: unknown) => error === failure
    )
    assert.equal(reads, 1)
    assert.equal(toasts, 0)
    assert.equal(useAuthStore.getState().auth.user?.id, 7)
  }
})

test('a successful transport still rejects missing, failed or malformed history payloads', async () => {
  login()
  for (const payload of [
    null,
    {},
    { success: false, data: emptyPage().data },
    { success: true },
    { success: true, data: { items: [] } },
    { success: true, data: { ...emptyPage().data, items: [{}] } },
  ]) {
    api.defaults.adapter = async (config) => response(config, payload)
    await assert.rejects(
      readLogsPage({ category: 'common', params: {}, isAdmin: false }),
      /history response unavailable/
    )
  }
})

test('missing authentication and already aborted reads never dispatch a request', async () => {
  let reads = 0
  api.defaults.adapter = async (config) => {
    reads += 1
    return response(config, emptyPage())
  }
  useAuthStore.getState().auth.reset('complete')
  await assert.rejects(
    readLogsPage({ category: 'common', params: {}, isAdmin: false }),
    /history session unavailable/
  )
  login()
  const controller = new AbortController()
  controller.abort()
  await assert.rejects(
    readLogsPage({
      category: 'common',
      params: {},
      isAdmin: false,
      signal: controller.signal,
    }),
    /history session changed/
  )
  assert.equal(reads, 0)
})

test('user, Session, role and sign-out changes reject successful late responses', async () => {
  const changeScope = [
    () => login({ id: 8 }),
    () => login({ sid: 'synthetic-history-replacement-session' }),
    () => login({ role: 10 }),
    () => useAuthStore.getState().auth.reset('complete'),
  ]
  for (const change of changeScope) {
    login()
    let reads = 0
    api.defaults.adapter = async (config) => {
      reads += 1
      change()
      return response(config, emptyPage())
    }
    await assert.rejects(
      readLogsPage({ category: 'common', params: {}, isAdmin: false }),
      /history session changed/
    )
    assert.equal(reads, 1)
  }
})

test('a frozen query scope cannot request another user, Session or role before dispatch', async () => {
  const changeScope = [
    () => login({ id: 8 }),
    () => login({ sid: 'synthetic-history-replacement-session' }),
    () => login({ role: 10 }),
    () => useAuthStore.getState().auth.reset('complete'),
  ]
  let reads = 0
  api.defaults.adapter = async (config) => {
    reads += 1
    return response(config, emptyPage())
  }
  for (const change of changeScope) {
    login()
    const scope = readUsageStatisticsScope()
    assert.ok(scope)
    change()
    await assert.rejects(
      readLogsPage({ category: 'common', params: {}, isAdmin: false, scope }),
      /history session changed/
    )
  }
  assert.equal(reads, 0)
})

test('scope changes also supersede an old unavailable or denial response', async () => {
  for (const status of [401, 403, 503]) {
    login()
    api.defaults.adapter = async (config) => {
      login({ role: 10 })
      throw new AxiosError(
        'Synthetic late failure',
        'ERR_BAD_RESPONSE',
        config,
        undefined,
        response(config, unavailable('drawing'), status)
      )
    }
    await assert.rejects(
      readLogsPage({ category: 'drawing', params: {}, isAdmin: false }),
      /history session changed/
    )
  }
})

test('cancellation during a read rejects both success and structured unavailable responses', async () => {
  for (const status of [200, 503]) {
    login()
    const controller = new AbortController()
    let reads = 0
    api.defaults.adapter = async (config) => {
      reads += 1
      controller.abort()
      if (status === 503) {
        throw new AxiosError(
          'Synthetic cancelled unavailable',
          'ERR_BAD_RESPONSE',
          config,
          undefined,
          response(config, unavailable('drawing'), status)
        )
      }
      return response(config, emptyPage())
    }
    await assert.rejects(
      readLogsPage({
        category: 'drawing',
        params: {},
        isAdmin: false,
        signal: controller.signal,
      }),
      /history session changed/
    )
    assert.equal(reads, 1)
  }
})
