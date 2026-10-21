/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, afterEach, beforeEach, test } from 'node:test'

import axios, { AxiosError, type AxiosAdapter } from 'axios'
import { toast } from 'sonner'

import { api } from '@/lib/api'
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
import { useAuthStore, type AuthBundle } from '@/stores/auth-store'

import { startTelegramLogin, telegramLogin } from './api'
import type { TelegramAuthorization } from './lib/telegram-login'

const descriptors = new Map(
  ['window', 'navigator', 'isSecureContext', 'BroadcastChannel'].map((key) => [
    key,
    Object.getOwnPropertyDescriptor(globalThis, key),
  ])
)
const storage = new Map<string, string>()
const lockNames: string[] = []
let tail: Promise<void> = Promise.resolve()
let held = 0
const locks = {
  async request<T>(
    name: string,
    options: { signal?: AbortSignal },
    callback: () => Promise<T>
  ) {
    lockNames.push(name)
    const predecessor = tail
    const released = deferred()
    tail = released.promise
    await predecessor
    try {
      if (options.signal?.aborted) {
        throw new DOMException('Aborted', 'AbortError')
      }
      held++
      try {
        return await callback()
      } finally {
        held--
      }
    } finally {
      released.resolve()
    }
  },
}
Object.defineProperty(globalThis, 'isSecureContext', {
  configurable: true,
  value: true,
})
Object.defineProperty(globalThis, 'navigator', {
  configurable: true,
  value: { locks },
})
Object.defineProperty(globalThis, 'BroadcastChannel', {
  configurable: true,
  value: undefined,
})
Object.defineProperty(globalThis, 'window', {
  configurable: true,
  value: {
    location: {
      origin: 'https://telegram-fixture.example.test',
      pathname: '/sign-in',
    },
    localStorage: {
      getItem: (key: string) => storage.get(key) ?? null,
      setItem: (key: string, value: string) => storage.set(key, value),
      removeItem: (key: string) => storage.delete(key),
    },
  },
})

const originalAdapter = api.defaults.adapter
const originalToast = toast.error
const originalRefreshAdapter = axios.getAdapter(originalAdapter)
const notifications: unknown[] = []
let refreshes = 0

function deferred() {
  let resolve: () => void = () => {}
  const promise = new Promise<void>((done) => {
    resolve = done
  })
  return { promise, resolve }
}
function response(
  config: Parameters<AxiosAdapter>[0],
  data: unknown,
  status = 200
) {
  return { config, data, status, statusText: 'synthetic response', headers: {} }
}
function bundle(
  token = 'synthetic-telegram-access',
  id = 42,
  sid = 'synthetic-telegram-session'
): AuthBundle {
  return {
    access_token: token,
    token_type: 'Bearer',
    access_expires_at: Math.floor(Date.now() / 1000) + 600,
    user: { id, username: `synthetic-${id}`, role: 1 },
    session: {
      sid,
      current: true,
      login_method: 'oauth:telegram',
      ip: '',
      user_agent: '',
      created_at: 1,
      last_active_at: 1,
      expires_at: 2_000_000_000,
    },
  }
}
const authorization = {
  id: '424242',
  auth_date: String(Math.floor(Date.now() / 1000)),
  hash: 'a'.repeat(64),
  first_name: 'Synthetic',
} satisfies TelegramAuthorization
const flowToken = `lmmt_${'b'.repeat(64)}`
const mfaToken = `lmmf_${'c'.repeat(64)}`

function assertCookieRequest(config: Parameters<AxiosAdapter>[0]) {
  assert.equal(held, 1)
  assert.ok([...storage.values()].some((value) => value.startsWith('pending:')))
  assert.equal(config.withCredentials, true)
  assert.equal(config.skipAuthRefresh, true)
  assert.equal(config.skipBusinessError, true)
  assert.equal(config.skipErrorHandler, true)
  assert.equal(config.timeout, AUTH_COOKIE_TIMEOUT_MS)
  assert.equal(
    config.signal,
    undefined,
    'view cancellation must not abort an active Cookie writer'
  )
}

beforeEach(() => {
  clearAuthentication(false, 'idle')
  notifications.length = 0
  refreshes = 0
  toast.error = (message) => {
    notifications.push(message)
    return 'telegram-api-test'
  }
  api.defaults.adapter = async () => {
    throw new Error('Unexpected Telegram API request')
  }
  setDevelopmentAuthRefreshAdapter(async () => {
    refreshes++
    throw new Error('Telegram login must not refresh or replay')
  })
})
afterEach(async () => {
  await tail
  assert.equal(held, 0)
  assert.ok(lockNames.length > 0)
  assert.ok(lockNames.every((name) => name === AUTH_COOKIE_LOCK))
  lockNames.length = 0
  assert.deepEqual(notifications, [])
  assert.equal(refreshes, 0)
  api.defaults.adapter = originalAdapter
  setDevelopmentAuthRefreshAdapter(originalRefreshAdapter)
  toast.error = originalToast
  clearAuthentication(false, 'idle')
  assert.equal(storage.size, 0)
})
after(() => {
  for (const [key, descriptor] of descriptors) {
    if (descriptor) Object.defineProperty(globalThis, key, descriptor)
    else Reflect.deleteProperty(globalThis, key)
  }
})

test('Telegram start validates the flow and expiry inside the Cookie lock without creating a session', async () => {
  const expiresAt = Math.floor(Date.now() / 1000) + 300
  let requests = 0
  api.defaults.adapter = async (config) => {
    requests++
    assertCookieRequest(config)
    assert.equal(config.method, 'post')
    assert.equal(config.url, '/api/oauth/telegram/login/start')
    assert.equal(config.data, undefined)
    return response(config, {
      success: true,
      data: {
        flow_token: flowToken,
        expires_at: expiresAt,
        ignored: 'not a flow field',
      },
    })
  }
  assert.deepEqual(await startTelegramLogin(new AbortController().signal), {
    flow_token: flowToken,
    expires_at: expiresAt,
  })
  assert.equal(requests, 1)
  assert.equal(useAuthStore.getState().auth.user, null)
  assert.equal(useAuthStore.getState().auth.pending2FAFlowToken, null)
})

test('Telegram callback commits its validated bundle before the next Cookie writer enters', async () => {
  const entered = deferred(),
    release = deferred()
  const expected = bundle()
  api.defaults.adapter = async (config) => {
    assertCookieRequest(config)
    assert.equal(config.method, 'get')
    assert.equal(config.url, '/api/oauth/telegram/login')
    assert.equal(config.disableDuplicate, true)
    assert.deepEqual(config.params, { ...authorization, state: flowToken })
    entered.resolve()
    await release.promise
    return response(config, { success: true, data: expected })
  }
  const attempt = telegramLogin(authorization, flowToken)
  await entered.promise
  let nextEntered = false
  const next = withAuthCookieLock(async () => {
    nextEntered = true
    assert.deepEqual(useAuthStore.getState().auth.user, expected.user)
    assert.equal(
      useAuthStore.getState().auth.accessToken,
      expected.access_token
    )
    assert.equal(storage.size, 0)
  })
  await Promise.resolve()
  assert.equal(nextEntered, false)
  release.resolve()
  const result = await attempt
  await next
  assert.deepEqual(result, { success: true, data: expected })
  assert.equal(useAuthStore.getState().auth.session?.sid, expected.session.sid)
})

test('Telegram require_2fa records only the pending challenge and never creates a session', async () => {
  const challenge = {
    require_2fa: true,
    flow_token: mfaToken,
    expires_at: Math.floor(Date.now() / 1000) + 300,
  }
  api.defaults.adapter = async (config) => {
    assertCookieRequest(config)
    return response(config, { success: true, data: challenge })
  }
  assert.deepEqual(await telegramLogin(authorization, flowToken), {
    success: true,
    data: challenge,
  })
  const auth = useAuthStore.getState().auth
  assert.equal(auth.pending2FAFlowToken, mfaToken)
  assert.equal(auth.pending2FAFlowExpiresAt, challenge.expires_at)
  assert.equal(auth.user, null)
  assert.equal(auth.session, null)
  assert.equal(auth.accessToken, null)
})

test('an explicit Telegram MFA challenge takes precedence over attached session fields', async () => {
  const challenge = {
    require_2fa: true,
    flow_token: mfaToken,
    expires_at: Math.floor(Date.now() / 1000) + 300,
  }
  api.defaults.adapter = async (config) => {
    assertCookieRequest(config)
    return response(config, {
      success: true,
      data: { ...bundle('synthetic-must-not-commit'), ...challenge },
    })
  }
  assert.deepEqual(await telegramLogin(authorization, flowToken), {
    success: true,
    data: challenge,
  })
  const auth = useAuthStore.getState().auth
  assert.equal(auth.pending2FAFlowToken, mfaToken)
  assert.equal(auth.pending2FAFlowExpiresAt, challenge.expires_at)
  assert.equal(auth.user, null)
  assert.equal(auth.session, null)
  assert.equal(auth.accessToken, null)
})

test('a malformed explicit Telegram MFA challenge never falls back to an attached session bundle', async () => {
  api.defaults.adapter = async (config) => {
    assertCookieRequest(config)
    return response(config, {
      success: true,
      data: {
        ...bundle('synthetic-must-not-commit'),
        require_2fa: true,
        flow_token: flowToken,
        expires_at: Math.floor(Date.now() / 1000) + 300,
      },
    })
  }
  await assert.rejects(
    telegramLogin(authorization, flowToken),
    (error: unknown) => {
      assert.ok(error instanceof Error)
      assert.equal(error.message, 'Login failed')
      assert.equal('code' in error, false)
      return true
    }
  )
  const auth = useAuthStore.getState().auth
  assert.equal(auth.pending2FAFlowToken, null)
  assert.equal(auth.user, null)
  assert.equal(auth.session, null)
  assert.equal(auth.accessToken, null)
})

test('Telegram cancellation sends nothing while queued and retains the lock through an active response commit', async () => {
  const blocked = deferred(),
    blockerEntered = deferred()
  const blocker = withAuthCookieLock(async () => {
    blockerEntered.resolve()
    await blocked.promise
  })
  await blockerEntered.promise
  let requests = 0
  api.defaults.adapter = async (config) => {
    requests++
    return response(config, { success: true, data: bundle() })
  }
  const waiting = new AbortController()
  const cancelled = assert.rejects(
    telegramLogin(authorization, flowToken, waiting.signal),
    (error: unknown) =>
      error instanceof DOMException && error.name === 'AbortError'
  )
  waiting.abort()
  blocked.resolve()
  await Promise.all([blocker, cancelled])
  assert.equal(requests, 0)

  const entered = deferred(),
    release = deferred(),
    active = new AbortController()
  const expected = bundle('synthetic-active-access')
  api.defaults.adapter = async (config) => {
    requests++
    assertCookieRequest(config)
    entered.resolve()
    await release.promise
    return response(config, { success: true, data: expected })
  }
  const activeCancelled = assert.rejects(
    telegramLogin(authorization, flowToken, active.signal),
    (error: unknown) => axios.isCancel(error)
  )
  await entered.promise
  active.abort()
  let nextEntered = false
  const next = withAuthCookieLock(async () => {
    nextEntered = true
    assert.equal(
      useAuthStore.getState().auth.accessToken,
      expected.access_token
    )
    assert.equal(storage.size, 0)
  })
  await Promise.resolve()
  assert.equal(nextEntered, false)
  release.resolve()
  await Promise.all([activeCancelled, next])
  assert.equal(requests, 1)
  assert.equal(useAuthStore.getState().auth.session?.sid, expected.session.sid)
})

test('a late Telegram callback cannot replace a newer account epoch', async () => {
  const entered = deferred(),
    release = deferred()
  const newer = bundle('synthetic-B-access', 43, 'synthetic-B-session')
  api.defaults.adapter = async (config) => {
    assertCookieRequest(config)
    entered.resolve()
    await release.promise
    return response(config, {
      success: true,
      data: bundle('synthetic-stale-A-access'),
    })
  }
  const cancelled = assert.rejects(
    telegramLogin(authorization, flowToken),
    (error: unknown) => axios.isCancel(error)
  )
  await entered.promise
  applyAuthBundle(newer, false)
  release.resolve()
  await cancelled
  const auth = useAuthStore.getState().auth
  assert.equal(auth.user?.id, newer.user.id)
  assert.equal(auth.accessToken, newer.access_token)
  assert.equal(auth.session?.sid, newer.session.sid)
  assert.equal(auth.pending2FAFlowToken, null)
})

test('Telegram 401 and business failures do not refresh, replay or emit central notifications', async () => {
  const existing = bundle('synthetic-existing-access')
  applyAuthBundle(existing, false)
  let requests = 0
  api.defaults.adapter = async (config) => {
    requests++
    assertCookieRequest(config)
    assert.equal(
      config.headers.get('Authorization'),
      `Bearer ${existing.access_token}`
    )
    if (requests === 1) {
      throw new AxiosError(
        'synthetic private diagnostic',
        'ERR_BAD_REQUEST',
        config,
        undefined,
        response(
          config,
          {
            success: false,
            code: 'TELEGRAM_LOGIN_INVALID',
            message: 'synthetic private diagnostic',
          },
          401
        )
      )
    }
    return response(config, {
      success: false,
      error: {
        code: 'TELEGRAM_LOGIN_INVALID',
        message: 'synthetic private diagnostic',
      },
      message: 'synthetic private diagnostic',
    })
  }
  await assert.rejects(
    telegramLogin(authorization, flowToken),
    (error: unknown) =>
      error instanceof AxiosError && error.response?.status === 401
  )
  assert.deepEqual(await telegramLogin(authorization, flowToken), {
    success: false,
    error: { code: 'TELEGRAM_LOGIN_INVALID' },
  })
  assert.equal(requests, 2, 'one dispatch per explicit attempt; never replay')
  assert.equal(useAuthStore.getState().auth.accessToken, existing.access_token)
  assert.equal(useAuthStore.getState().auth.session?.sid, existing.session.sid)
})

test('Telegram start rejects malformed or expired flow data without creating authentication state', async () => {
  const future = Math.floor(Date.now() / 1000) + 300
  const invalidFlows = [
    { flow_token: 'not-a-Telegram-flow', expires_at: future },
    { flow_token: flowToken, expires_at: 0 },
    { flow_token: flowToken, expires_at: Math.floor(Date.now() / 1000) - 1 },
  ]
  let requests = 0
  for (const invalid of invalidFlows) {
    api.defaults.adapter = async (config) => {
      requests++
      assertCookieRequest(config)
      return response(config, { success: true, data: invalid })
    }
    await assert.rejects(startTelegramLogin())
    assert.equal(useAuthStore.getState().auth.user, null)
    assert.equal(useAuthStore.getState().auth.pending2FAFlowToken, null)
  }
  assert.equal(requests, invalidFlows.length)
})

test('Telegram callback rejects malformed or expired MFA challenges without recording a pending flow', async () => {
  const future = Math.floor(Date.now() / 1000) + 300
  const invalidChallenges = [
    {
      data: { require_2fa: true, expires_at: future },
      message: 'Login failed',
      code: null,
    },
    {
      data: { require_2fa: true, flow_token: flowToken, expires_at: future },
      message: 'Login failed',
      code: null,
    },
    {
      data: {
        require_2fa: true,
        flow_token: mfaToken,
        expires_at: Math.floor(Date.now() / 1000) - 1,
      },
      message: 'Login flow expired',
      code: 'MFA_FLOW_EXPIRED',
    },
  ]
  for (const invalid of invalidChallenges) {
    api.defaults.adapter = async (config) => {
      assertCookieRequest(config)
      return response(config, { success: true, data: invalid.data })
    }
    await assert.rejects(
      telegramLogin(authorization, flowToken),
      (error: unknown) => {
        assert.ok(error instanceof Error)
        assert.equal(error.message, invalid.message)
        if (invalid.code === null) {
          assert.equal('code' in error, false)
        } else {
          assert.ok('code' in error)
          assert.equal(error.code, invalid.code)
        }
        return true
      }
    )
    const auth = useAuthStore.getState().auth
    assert.equal(auth.pending2FAFlowToken, null)
    assert.equal(auth.user, null)
    assert.equal(auth.session, null)
    assert.equal(auth.accessToken, null)
  }
})
