/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, afterEach, beforeEach, test } from 'node:test'

import {
  AxiosError,
  getAdapter,
  isCancel,
  type AxiosAdapter,
  type AxiosResponse,
} from 'axios'
import { toast } from 'sonner'
import { ZodError } from 'zod'

import {
  AUTH_COOKIE_LOCK,
  AUTH_COOKIE_TIMEOUT_MS,
  withAuthCookieLock,
} from '@/lib/auth-cookie'
import {
  applyAuthBundle,
  clearAuthentication,
  setDevelopmentAuthRefreshAdapter,
} from '@/lib/auth-session'
import { api } from '@/lib/http-client'
import { useAuthStore, type AuthBundle } from '@/stores/auth-store'

import { getUserProfile, startTelegramBind } from './api'

const origin = 'https://console.example.test'
const now = 2_000_000_000
const token = `lmmt_${'a'.repeat(64)}`
const originalNow = Date.now
const originalAdapter = api.defaults.adapter
const originalRefreshAdapter = getAdapter(originalAdapter)
const originalToastError = toast.error
const descriptors = new Map(
  ['window', 'navigator', 'isSecureContext'].map((key) => [
    key,
    Object.getOwnPropertyDescriptor(globalThis, key),
  ])
)
const storage = new Map<string, string>()
const lockNames: string[] = []
const errorToasts: unknown[] = []
let held = 0
let maxHeld = 0
let tail: Promise<unknown> = Promise.resolve()

function deferred() {
  let resolve!: () => void
  const promise = new Promise<void>((done) => {
    resolve = done
  })
  return { promise, resolve }
}

Object.defineProperty(globalThis, 'isSecureContext', {
  configurable: true,
  value: true,
})
Object.defineProperty(globalThis, 'navigator', {
  configurable: true,
  value: {
    locks: {
      async request<T>(
        name: string,
        options: { mode: string; signal?: AbortSignal },
        callback: () => Promise<T>
      ): Promise<T> {
        assert.equal(options.mode, 'exclusive')
        lockNames.push(name)
        const predecessor = tail
        const release = deferred()
        tail = release.promise
        await predecessor
        try {
          if (options.signal?.aborted) {
            throw new DOMException('Aborted', 'AbortError')
          }
          held++
          maxHeld = Math.max(maxHeld, held)
          try {
            return await callback()
          } finally {
            held--
          }
        } finally {
          release.resolve()
        }
      },
    },
  },
})
Object.defineProperty(globalThis, 'window', {
  configurable: true,
  value: {
    location: { origin, pathname: '/profile' },
    localStorage: {
      getItem: (key: string) => storage.get(key) ?? null,
      setItem: (key: string, value: string) => storage.set(key, value),
      removeItem: (key: string) => storage.delete(key),
    },
  },
})

function bundle(
  accessToken = 'initial',
  userId = 42,
  sessionId = 'session-a'
): AuthBundle {
  return {
    access_token: accessToken,
    token_type: 'Bearer',
    access_expires_at: now + 600,
    user: { id: userId, username: 'synthetic', role: 1 },
    session: {
      sid: sessionId,
      current: true,
      login_method: 'password',
      ip: '',
      user_agent: '',
      created_at: now - 100,
      last_active_at: now,
      expires_at: now + 3600,
    },
  }
}

function flow() {
  return {
    flow_token: token,
    callback_url: `${origin}/api/oauth/telegram/bind/${token}`,
    expires_at: now + 300,
  }
}

function response(
  config: Parameters<AxiosAdapter>[0],
  status: number,
  data: unknown
): AxiosResponse {
  return { config, status, statusText: 'fixture', headers: {}, data }
}

function pendingMarker() {
  const marker = storage.get('new-api:auth-cookie:uncertain')
  assert.ok(marker)
  assert.ok(marker.startsWith('pending:'))
  return marker
}

beforeEach(() => {
  Date.now = () => now * 1000
  applyAuthBundle(bundle(), false)
  setDevelopmentAuthRefreshAdapter(async () => {
    throw new Error('Unexpected refresh in this focused fixture')
  })
  toast.error = (...args: Parameters<typeof toast.error>) => {
    errorToasts.push(args[0])
    return 'telegram-focused-test'
  }
})

afterEach(async () => {
  await tail
  assert.equal(held, 0)
  assert.ok(maxHeld <= 1)
  assert.ok(lockNames.every((name) => name === AUTH_COOKIE_LOCK))
  assert.equal(storage.size, 0)
  assert.deepEqual(errorToasts, [])
  api.defaults.adapter = originalAdapter
  setDevelopmentAuthRefreshAdapter(originalRefreshAdapter)
  toast.error = originalToastError
  Date.now = originalNow
  clearAuthentication(false, 'idle')
  lockNames.length = 0
  errorToasts.length = 0
  maxHeld = 0
})

after(() => {
  for (const [key, descriptor] of descriptors) {
    if (descriptor) Object.defineProperty(globalThis, key, descriptor)
    else Reflect.deleteProperty(globalThis, key)
  }
})

test('starts with no body, Cookie writer intent and scoped Bearer without rotating authentication', async () => {
  let requests = 0
  api.defaults.adapter = async (config) => {
    requests++
    assert.equal(config.url, '/api/oauth/telegram/bind/start')
    assert.equal(config.method, 'post')
    assert.equal(config.data, undefined)
    assert.equal(config.withCredentials, true)
    assert.equal(config.timeout, AUTH_COOKIE_TIMEOUT_MS)
    assert.equal(config.skipAuthRefresh, true)
    assert.equal(config.skipBusinessError, true)
    assert.equal(config.skipErrorHandler, true)
    assert.equal(config.acceptAuthRotation, undefined)
    assert.deepEqual(config.authScope, {
      userId: 42,
      sessionId: 'session-a',
    })
    assert.equal(config.headers.get('Authorization'), 'Bearer initial')
    assert.equal(config.headers.get('X-Auth-Session'), 'session-a')
    assert.equal(held, 1)
    pendingMarker()
    return response(config, 200, { success: true, data: flow() })
  }

  assert.deepEqual(await startTelegramBind(), {
    success: true,
    data: flow(),
  })
  assert.equal(requests, 1)
  assert.equal(lockNames.length, 1)
  assert.equal(useAuthStore.getState().auth.accessToken, 'initial')
  assert.equal(useAuthStore.getState().auth.session?.sid, 'session-a')
})

test('Cookie-only start does not invent optional Bearer or X-Auth-Session headers', async () => {
  clearAuthentication(false, 'idle')
  api.defaults.adapter = async (config) => {
    assert.equal(config.withCredentials, true)
    assert.equal(config.headers.get('Authorization'), undefined)
    assert.equal(config.headers.get('X-Auth-Session'), undefined)
    assert.equal(held, 1)
    pendingMarker()
    return response(config, 200, { success: true, data: flow() })
  }
  assert.equal((await startTelegramBind()).success, true)
})

test('concurrent starts serialize dispatched writers and retain separate pending intents', async () => {
  const entered = deferred()
  const release = deferred()
  const markers: string[] = []
  let requests = 0
  api.defaults.adapter = async (config) => {
    requests++
    assert.equal(held, 1)
    markers.push(pendingMarker())
    if (requests === 1) {
      entered.resolve()
      await release.promise
    }
    return response(config, 200, { success: true, data: flow() })
  }

  const first = startTelegramBind()
  await entered.promise
  const second = startTelegramBind()
  try {
    await Promise.resolve()
    assert.equal(requests, 1)
    assert.equal(lockNames.length, 2)
    assert.equal(held, 1)
    assert.equal(pendingMarker(), markers[0])
  } finally {
    release.resolve()
  }
  await Promise.all([first, second])
  assert.equal(requests, 2)
  assert.equal(markers.length, 2)
  assert.notEqual(markers[0], markers[1])
})

test('refreshes an expired session before acquiring the start writer lock without nesting', async () => {
  const expired = bundle()
  expired.access_expires_at = now - 1
  applyAuthBundle(expired, false)
  const order: string[] = []
  setDevelopmentAuthRefreshAdapter(async (config) => {
    order.push('refresh')
    assert.equal(held, 1)
    pendingMarker()
    assert.equal(config.url, '/api/user/auth/refresh')
    assert.equal(config.headers.get('X-Auth-Session'), 'session-a')
    return response(config, 200, { success: true, data: bundle('prepared') })
  })
  api.defaults.adapter = async (config) => {
    order.push('start')
    assert.equal(held, 1)
    pendingMarker()
    assert.equal(config.headers.get('Authorization'), 'Bearer prepared')
    assert.equal(config.skipAuthRefresh, true)
    return response(config, 200, { success: true, data: flow() })
  }

  await startTelegramBind()
  assert.deepEqual(order, ['refresh', 'start'])
  assert.equal(lockNames.length, 2)
  assert.equal(useAuthStore.getState().auth.accessToken, 'prepared')
})

test('a queued start never dispatches as a replacement account', async () => {
  const entered = deferred()
  const release = deferred()
  const blocker = withAuthCookieLock(async () => {
    entered.resolve()
    await release.promise
    applyAuthBundle(bundle('account-b', 99, 'session-b'), false)
  })
  await entered.promise
  let requests = 0
  api.defaults.adapter = async (config) => {
    requests++
    return response(config, 200, { success: true, data: flow() })
  }
  const rejected = assert.rejects(startTelegramBind(), isCancel)
  release.resolve()
  await Promise.all([blocker, rejected])
  assert.equal(requests, 0)
  assert.equal(useAuthStore.getState().auth.user?.id, 99)
})

test('a rejected start refreshes only after releasing its lock and never replays the start', async () => {
  let starts = 0
  let refreshes = 0
  api.defaults.adapter = async (config) => {
    starts++
    assert.equal(held, 1)
    pendingMarker()
    throw new AxiosError(
      'synthetic private transport detail',
      'ERR_BAD_REQUEST',
      config,
      undefined,
      response(config, 401, { code: 'TELEGRAM_BIND_SESSION_INVALID' })
    )
  }
  setDevelopmentAuthRefreshAdapter(async (config) => {
    refreshes++
    assert.equal(held, 1)
    return response(config, 200, { success: true, data: bundle('refreshed') })
  })

  await assert.rejects(
    startTelegramBind(),
    (error: unknown) =>
      error instanceof AxiosError && error.response?.status === 401
  )
  assert.equal(starts, 1)
  assert.equal(refreshes, 1)
  assert.equal(lockNames.length, 2)
  assert.equal(useAuthStore.getState().auth.accessToken, 'refreshed')
})

for (const replacement of [
  { name: 'another account and session', bundle: bundle('b', 99, 'session-b') },
  { name: 'another account with the same SID', bundle: bundle('b', 99) },
  {
    name: 'the same account with a new SID',
    bundle: bundle('b', 42, 'session-b'),
  },
]) {
  test(`a late self response is cancelled after switching to ${replacement.name}`, async () => {
    const entered = deferred()
    const release = deferred()
    api.defaults.adapter = async (config) => {
      assert.equal(config.url, '/api/user/self')
      assert.deepEqual(config.authScope, {
        userId: 42,
        sessionId: 'session-a',
      })
      entered.resolve()
      await release.promise
      return response(config, 200, { success: true, data: { id: 42 } })
    }
    const rejected = assert.rejects(getUserProfile(), isCancel)
    await entered.promise
    applyAuthBundle(replacement.bundle, false)
    release.resolve()
    await rejected
    assert.equal(
      useAuthStore.getState().auth.accessToken,
      replacement.bundle.access_token
    )
  })
}

test('self read preserves local timeout and toast policy alongside its captured scope', async () => {
  const payload = { success: true, data: { id: 42, username: 'synthetic' } }
  api.defaults.adapter = async (config) => {
    assert.equal(config.timeout, 3141)
    assert.equal(config.skipBusinessError, true)
    assert.equal(config.skipErrorHandler, true)
    assert.deepEqual(config.authScope, {
      userId: 42,
      sessionId: 'session-a',
    })
    return response(config, 200, payload)
  }
  assert.deepEqual(
    await getUserProfile({
      timeout: 3141,
      skipBusinessError: true,
      skipErrorHandler: true,
    }),
    payload
  )
})

for (const expiresAt of [now + 1, now + 300]) {
  test(`accepts the exact valid lifetime boundary ${expiresAt - now} seconds`, async () => {
    const data = { ...flow(), expires_at: expiresAt }
    api.defaults.adapter = async (config) =>
      response(config, 200, { success: true, data })
    assert.deepEqual(await startTelegramBind(), { success: true, data })
  })
}

test('accepts a server-issued 300-second flow when the browser clock is one second behind', async () => {
  const serverNow = now + 1
  const data = { ...flow(), expires_at: serverNow + 300 }
  assert.equal(data.expires_at, now + 301)
  api.defaults.adapter = async (config) =>
    response(config, 200, { success: true, data })
  assert.deepEqual(await startTelegramBind(), { success: true, data })
})

const invalidContracts: Array<{ name: string; payload: unknown }> = [
  { name: 'null envelope', payload: null },
  { name: 'string success', payload: { success: 'true', data: flow() } },
  { name: 'numeric success', payload: { success: 1, data: flow() } },
  { name: 'null success', payload: { success: null, data: flow() } },
  { name: 'missing success', payload: { data: flow() } },
  { name: 'missing flow', payload: { success: true } },
  { name: 'null flow', payload: { success: true, data: null } },
  ...[
    {
      name: 'wrong flow prefix',
      data: { ...flow(), flow_token: 'x'.repeat(64) },
    },
    {
      name: 'short flow',
      data: { ...flow(), flow_token: `lmmt_${'a'.repeat(63)}` },
    },
    {
      name: 'long flow',
      data: { ...flow(), flow_token: `lmmt_${'a'.repeat(65)}` },
    },
    {
      name: 'nonhex flow',
      data: { ...flow(), flow_token: `lmmt_${'g'.repeat(64)}` },
    },
    { name: 'numeric flow', data: { ...flow(), flow_token: 1 } },
    { name: 'expired lifetime', data: { ...flow(), expires_at: now } },
    { name: 'past lifetime', data: { ...flow(), expires_at: now - 1 } },
    { name: 'fractional lifetime', data: { ...flow(), expires_at: now + 1.5 } },
    {
      name: 'string lifetime',
      data: { ...flow(), expires_at: String(now + 1) },
    },
    { name: 'NaN lifetime', data: { ...flow(), expires_at: Number.NaN } },
    { name: 'infinite lifetime', data: { ...flow(), expires_at: Infinity } },
    {
      name: 'external callback',
      data: {
        ...flow(),
        callback_url: `https://other.example.test/api/oauth/telegram/bind/${token}`,
      },
    },
    {
      name: 'query callback',
      data: { ...flow(), callback_url: `${flow().callback_url}?x=1` },
    },
    {
      name: 'hash callback',
      data: { ...flow(), callback_url: `${flow().callback_url}#result` },
    },
    {
      name: 'credential callback',
      data: {
        ...flow(),
        callback_url: `https://synthetic:credential@console.example.test/api/oauth/telegram/bind/${token}`,
      },
    },
    {
      name: 'relative callback',
      data: { ...flow(), callback_url: `/api/oauth/telegram/bind/${token}` },
    },
    {
      name: 'normalized equivalent host callback',
      data: {
        ...flow(),
        callback_url: `https://CONSOLE.example.test/api/oauth/telegram/bind/${token}`,
      },
    },
    {
      name: 'normalized equivalent port callback',
      data: {
        ...flow(),
        callback_url: `https://console.example.test:443/api/oauth/telegram/bind/${token}`,
      },
    },
    {
      name: 'trailing slash callback',
      data: { ...flow(), callback_url: `${flow().callback_url}/` },
    },
    {
      name: 'another flow callback',
      data: {
        ...flow(),
        callback_url: `${origin}/api/oauth/telegram/bind/lmmt_${'b'.repeat(64)}`,
      },
    },
    { name: 'nonstring callback', data: { ...flow(), callback_url: false } },
  ].map(({ name, data }) => ({ name, payload: { success: true, data } })),
  { name: 'nonstring failure code', payload: { success: false, code: true } },
  {
    name: 'nonstring nested failure code',
    payload: { success: false, error: { code: 1 } },
  },
  {
    name: 'overlong failure code',
    payload: { success: false, code: 'x'.repeat(129) },
  },
]

for (const { name, payload } of invalidContracts) {
  test(`rejects ${name} instead of accepting a malformed bind contract`, async () => {
    api.defaults.adapter = async (config) => response(config, 200, payload)
    await assert.rejects(startTelegramBind(), ZodError)
    assert.equal(useAuthStore.getState().auth.accessToken, 'initial')
  })
}

for (const failure of [
  { success: false, code: 'TELEGRAM_BIND_DISABLED' },
  { success: false, error: { code: 'TELEGRAM_BIND_SESSION_INVALID' } },
  { success: false, code: 'UNKNOWN_BIND_CODE' },
]) {
  test(`preserves only structured failure code ${JSON.stringify(failure)} without raw server messages`, async () => {
    api.defaults.adapter = async (config) =>
      response(config, 200, {
        ...failure,
        message: 'synthetic private provider detail',
        data: { private_detail: 'synthetic private data' },
        ...(failure.error
          ? {
              error: {
                ...failure.error,
                message: 'synthetic nested provider detail',
              },
            }
          : {}),
      })
    assert.deepEqual(await startTelegramBind(), failure)
    assert.equal(useAuthStore.getState().auth.accessToken, 'initial')
  })
}
