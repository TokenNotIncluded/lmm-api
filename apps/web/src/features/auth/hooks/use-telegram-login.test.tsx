/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
// @ts-expect-error Bun's test module is available only in the test runtime.
import { mock } from 'bun:test'
import assert from 'node:assert/strict'
import { after, afterEach, beforeEach, test } from 'node:test'

import { AxiosError } from 'axios'
import { Window } from 'happy-dom'
import i18next from 'i18next'
import type { ReactNode } from 'react'

import type { AuthBundle } from '@/stores/auth-store'

const unexpectedRequests: string[] = []
const dom = new Window({
  url: 'https://console.example.test/sign-in',
  settings: {
    disableJavaScriptFileLoading: true,
    fetch: {
      interceptor: {
        beforeAsyncRequest: async ({ request }) => {
          unexpectedRequests.push(request.url)
          throw new Error('Unexpected network request')
        },
      },
    },
  },
})
const descriptors = new Map<string, PropertyDescriptor | undefined>()
for (const name of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'SVGElement',
  'Node',
  'Element',
  'Event',
  'MouseEvent',
  'MutationObserver',
] as const) {
  descriptors.set(name, Object.getOwnPropertyDescriptor(globalThis, name))
  Object.defineProperty(globalThis, name, {
    configurable: true,
    value: dom[name],
  })
}
for (const [name, value] of [
  ['IS_REACT_ACT_ENVIRONMENT', true],
  ['BroadcastChannel', undefined],
] as const) {
  descriptors.set(name, Object.getOwnPropertyDescriptor(globalThis, name))
  Object.defineProperty(globalThis, name, { configurable: true, value })
}
const errors: string[] = []
const successes: string[] = []
const redirects: unknown[] = []
const completed: Array<{
  bundle: unknown
  redirect: unknown
  committed: unknown
  signal: unknown
}> = []
const t = (key: string, values?: Record<string, unknown>) =>
  Object.entries(values ?? {}).reduce(
    (text, [name, value]) => text.replaceAll(`{{${name}}}`, String(value)),
    key
  )
mock.module('react-i18next', () => ({ useTranslation: () => ({ t }) }))
mock.module('sonner', () => ({
  toast: {
    error: (message: string) => {
      errors.push(message)
      return 'telegram-error'
    },
    success: (message: string) => {
      successes.push(message)
      return 'telegram-success'
    },
  },
}))
mock.module('@/components/dialog', () => ({
  Dialog: ({ open, children }: { open: boolean; children: ReactNode }) =>
    open ? <section>{children}</section> : null,
}))
mock.module('./use-auth-redirect', () => ({
  useAuthRedirect: () => ({
    redirectTo2FA: (redirect: unknown) => redirects.push(redirect),
    handleLoginSuccess: async (
      bundle: unknown,
      redirect: unknown,
      committed: unknown,
      signal: unknown
    ) => {
      completed.push({ bundle, redirect, committed, signal })
    },
  }),
}))

await i18next.init({ lng: 'en', resources: { en: { translation: {} } } })
const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { TelegramLoginDialog } =
  await import('../components/telegram-login-dialog')
const { useOAuthLogin } = await import('./use-oauth-login')
const { api } = await import('@/lib/api')
const { clearAuthentication } = await import('@/lib/auth-session')
const { useAuthStore } = await import('@/stores/auth-store')
const originalAdapter = api.defaults.adapter
let root: ReturnType<typeof createRoot> | undefined
let host: HTMLElement | undefined
let controls: ReturnType<typeof useOAuthLogin> | undefined
let flowNumber = 0
let callbackCalls = 0
let startGate: Promise<void> | undefined
let callbackGate: Promise<void> | undefined
let callbackPayload: unknown
let callbackStatus = 200
const paths: string[] = []
const assertion = {
  id: '9223372036854775807',
  auth_date: '1900000000',
  hash: 'aB'.repeat(32),
  first_name: ' e\u0301 ',
  last_name: '',
}
function bundle(id = 42): AuthBundle {
  return {
    access_token: `synthetic-token-${id}`,
    token_type: 'Bearer',
    access_expires_at: Math.floor(Date.now() / 1000) + 600,
    user: { id, username: `synthetic-${id}`, role: 1 },
    session: {
      sid: `synthetic-session-${id}`,
      current: true,
      login_method: 'telegram',
      ip: '',
      user_agent: '',
      created_at: 1,
      last_active_at: 1,
      expires_at: 2_000_000_000,
    },
  }
}
function deferred() {
  let resolve: () => void = () => {}
  const promise = new Promise<void>((done) => {
    resolve = done
  })
  return { promise, resolve }
}
function current() {
  assert.ok(controls)
  return controls
}
function Harness() {
  const flow = useOAuthLogin(
    { telegram_oauth: true, telegram_bot_name: '@native_test_bot' },
    '/dashboard',
    true
  )
  controls = flow
  return (
    <TelegramLoginDialog
      open={flow.isTelegramDialogOpen}
      botName='@native_test_bot'
      pending={flow.isTelegramPending}
      onOpenChange={flow.handleTelegramDialogChange}
      onAuthorization={flow.handleTelegramAuthorization}
    />
  )
}
async function mount() {
  host = document.createElement('div')
  document.body.append(host)
  root = createRoot(host)
  await act(async () => root?.render(<Harness />))
}
async function start() {
  await act(async () => current().handleTelegramLogin())
}
function widgetCallback() {
  const script = host?.querySelector('script[data-onauth]')
  assert.ok(script)
  assert.equal(script.getAttribute('data-telegram-login'), 'native_test_bot')
  const expression = script.getAttribute('data-onauth')
  assert.ok(expression)
  const match = /^newApiTelegramLogin([0-9]+)\(user\)$/.exec(expression)
  assert.ok(match)
  const name: `newApiTelegramLogin${number}` = `newApiTelegramLogin${Number(match[1])}`
  const callback = window[name]
  assert.equal(typeof callback, 'function')
  if (typeof callback !== 'function') throw new Error('Missing widget callback')
  return { callback, name }
}
beforeEach(() => {
  clearAuthentication(false, 'idle')
  callbackPayload = { success: true, data: bundle() }
  callbackStatus = 200
  callbackCalls = 0
  flowNumber = 0
  api.defaults.adapter = async (config) => {
    const path = config.url ?? ''
    paths.push(path)
    let data: unknown
    if (path === '/api/user/auth/logout') data = { success: true, message: '' }
    else if (path === '/api/oauth/telegram/login/start') {
      await startGate
      data = {
        success: true,
        data: {
          flow_token: `lmmt_${String(++flowNumber).padStart(64, '0')}`,
          expires_at: Math.floor(Date.now() / 1000) + 300,
        },
      }
    } else if (path === '/api/oauth/telegram/login') {
      callbackCalls++
      assert.deepEqual(config.params, {
        ...assertion,
        state: `lmmt_${String(flowNumber).padStart(64, '0')}`,
      })
      await callbackGate
      data = callbackPayload
    } else throw new Error(`Unexpected API: ${path}`)
    const response = {
      config,
      status: path === '/api/oauth/telegram/login' ? callbackStatus : 200,
      statusText: 'synthetic',
      headers: {},
      data,
    }
    if (response.status >= 400) {
      throw new AxiosError(
        'private transport diagnostic',
        'ERR_BAD_RESPONSE',
        config,
        undefined,
        response
      )
    }
    return response
  }
})
afterEach(async () => {
  const activeRoot = root
  if (activeRoot) await act(async () => activeRoot.unmount())
  root = undefined
  host = undefined
  controls = undefined
  api.defaults.adapter = originalAdapter
  clearAuthentication(false, 'idle')
  errors.length =
    successes.length =
    redirects.length =
    completed.length =
    paths.length =
      0
  startGate = callbackGate = undefined
  document.body.replaceChildren()
  assert.deepEqual(unexpectedRequests.splice(0), [])
})
after(() => {
  dom.close()
  for (const [name, descriptor] of descriptors) {
    if (descriptor) Object.defineProperty(globalThis, name, descriptor)
    else Reflect.deleteProperty(globalThis, name)
  }
})

test('native Telegram Bundle is already committed once before navigating', async () => {
  await mount()
  await start()
  await act(async () => current().handleTelegramAuthorization(assertion))
  assert.equal(callbackCalls, 1)
  assert.equal(completed.length, 1)
  assert.equal(completed[0]?.committed, true)
  assert.equal(completed[0]?.redirect, '/dashboard')
  assert.ok(completed[0]?.signal instanceof AbortSignal)
  assert.equal(useAuthStore.getState().auth.user?.id, 42)
  assert.deepEqual(redirects, [])
  assert.deepEqual(successes, ['Welcome back!'])
  assert.deepEqual(errors, [])
})

test('require_2fa uses the existing challenge route with no premature session or welcome', async () => {
  const challenge = {
    require_2fa: true,
    flow_token: `lmmf_${'b'.repeat(64)}`,
    expires_at: Math.floor(Date.now() / 1000) + 300,
  }
  callbackPayload = { success: true, data: challenge }
  await mount()
  await start()
  await act(async () => current().handleTelegramAuthorization(assertion))
  assert.deepEqual(redirects, ['/dashboard'])
  assert.equal(
    useAuthStore.getState().auth.pending2FAFlowToken,
    challenge.flow_token
  )
  assert.equal(useAuthStore.getState().auth.session, null)
  assert.deepEqual(completed, [])
  assert.deepEqual(successes, [])
  assert.deepEqual(errors, [])
})

test('same-tick duplicate widget callbacks submit only once', async () => {
  const gate = deferred()
  callbackGate = gate.promise
  await mount()
  await start()
  const { callback } = widgetCallback()
  await act(async () => {
    callback(assertion)
    callback(assertion)
    await Promise.resolve()
  })
  assert.equal(callbackCalls, 1)
  await act(async () => {
    gate.resolve()
    await new Promise((resolve) => setTimeout(resolve, 10))
  })
  assert.equal(completed.length, 1)
})

test('a mixed response cannot bypass a required second factor with bundled session fields', async () => {
  const challenge = {
    require_2fa: true,
    flow_token: `lmmf_${'b'.repeat(64)}`,
    expires_at: Math.floor(Date.now() / 1000) + 300,
  }
  callbackPayload = { success: true, data: { ...bundle(), ...challenge } }
  await mount()
  await start()
  await act(async () => current().handleTelegramAuthorization(assertion))
  assert.equal(useAuthStore.getState().auth.user, null)
  assert.equal(useAuthStore.getState().auth.session, null)
  assert.equal(
    useAuthStore.getState().auth.pending2FAFlowToken,
    challenge.flow_token
  )
  assert.deepEqual(redirects, ['/dashboard'])
  assert.deepEqual(completed, [])
  assert.deepEqual(successes, [])
})

test('closing and reopening creates a new widget identity and ignores retained old callbacks', async () => {
  await mount()
  await start()
  const old = widgetCallback()
  await act(async () => current().handleTelegramDialogChange(false))
  await start()
  const next = widgetCallback()
  assert.notEqual(old.name, next.name)
  assert.equal(window[old.name], undefined)
  await act(async () => old.callback(assertion))
  assert.equal(callbackCalls, 0)
  assert.deepEqual(completed, [])
})

test('closing an active Cookie response suppresses late navigation while the API commits its real result', async () => {
  const gate = deferred()
  callbackGate = gate.promise
  await mount()
  await start()
  let pending: Promise<void> | undefined
  await act(async () => {
    pending = current().handleTelegramAuthorization(assertion)
    await Promise.resolve()
  })
  assert.equal(callbackCalls, 1)
  await act(async () => current().handleTelegramDialogChange(false))
  await act(async () => {
    gate.resolve()
    await pending
  })
  assert.equal(useAuthStore.getState().auth.user?.id, 42)
  assert.deepEqual(completed, [])
  assert.deepEqual(redirects, [])
  assert.deepEqual(successes, [])
  assert.deepEqual(errors, [])
})

test('a callback after account change is ignored before any request', async () => {
  await mount()
  await start()
  useAuthStore.getState().auth.setBundle(bundle(43))
  await act(async () => current().handleTelegramAuthorization(assertion))
  assert.equal(callbackCalls, 0)
  assert.equal(useAuthStore.getState().auth.user?.id, 43)
  assert.deepEqual(completed, [])
  assert.deepEqual(successes, [])
})

test('closing an unfinished start cannot reopen its widget later', async () => {
  const gate = deferred()
  startGate = gate.promise
  await mount()
  let pending: Promise<void> | undefined
  await act(async () => {
    pending = current().handleTelegramLogin()
    await new Promise((resolve) => setTimeout(resolve, 5))
  })
  await act(async () => current().handleTelegramDialogChange(false))
  await act(async () => {
    gate.resolve()
    await pending
  })
  assert.equal(current().isTelegramDialogOpen, false)
  assert.equal(host?.querySelector('script'), null)
  assert.deepEqual(errors, [])
})

test('unknown rejected callback is safe and a new start is required before retry', async () => {
  callbackStatus = 401
  callbackPayload = {
    success: false,
    code: 'AUTH_UNAUTHORIZED',
    message: 'private diagnostic',
  }
  await mount()
  await start()
  await act(async () => current().handleTelegramAuthorization(assertion))
  await act(async () => current().handleTelegramAuthorization(assertion))
  assert.equal(callbackCalls, 1)
  assert.deepEqual(errors, ['Login failed'])
  assert.equal(paths.includes('/api/user/auth/refresh'), false)
  assert.deepEqual(completed, [])
  assert.deepEqual(successes, [])
})

test('expired MFA challenge never opens OTP or creates a session', async () => {
  callbackPayload = {
    success: true,
    data: {
      require_2fa: true,
      flow_token: `lmmf_${'b'.repeat(64)}`,
      expires_at: Math.floor(Date.now() / 1000) - 1,
    },
  }
  await mount()
  await start()
  await act(async () => current().handleTelegramAuthorization(assertion))
  assert.deepEqual(redirects, [])
  assert.equal(useAuthStore.getState().auth.pending2FAFlowToken, null)
  assert.equal(useAuthStore.getState().auth.user, null)
  assert.deepEqual(errors, [
    'This login flow has expired or has already been used. Sign in again.',
  ])
})
