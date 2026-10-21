/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import assert from 'node:assert/strict'
import { after, afterEach, beforeEach, test } from 'node:test'

import { AxiosError, type InternalAxiosRequestConfig } from 'axios'
import { Window } from 'happy-dom'

const domWindow = new Window({
  url: 'https://console.example.test/profile',
  settings: {
    enableJavaScriptEvaluation: false,
    disableJavaScriptFileLoading: true,
    disableCSSFileLoading: true,
  },
})
const descriptors = new Map<string, PropertyDescriptor | undefined>()
for (const key of [
  'window',
  'document',
  'navigator',
  'localStorage',
  'HTMLElement',
  'HTMLButtonElement',
  'HTMLInputElement',
  'SVGElement',
  'Node',
  'Element',
  'DocumentFragment',
  'Event',
  'MessageEvent',
  'MouseEvent',
  'PointerEvent',
  'FocusEvent',
  'CustomEvent',
  'MutationObserver',
  'ResizeObserver',
  'requestAnimationFrame',
  'cancelAnimationFrame',
  'getComputedStyle',
] as const) {
  descriptors.set(key, Object.getOwnPropertyDescriptor(globalThis, key))
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: domWindow[key],
  })
}
for (const [key, value] of [
  ['IS_REACT_ACT_ENVIRONMENT', true],
  ['isSecureContext', false],
] as const) {
  descriptors.set(key, Object.getOwnPropertyDescriptor(globalThis, key))
  Object.defineProperty(globalThis, key, { configurable: true, value })
}

const { act, useState } = await import('react')
const { createRoot } = await import('react-dom/client')
const { createInstance, default: globalI18n } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { toast } = await import('sonner')
const { api } = await import('@/lib/api')
const {
  AuthCookieBlockedError,
  authCookieRecoveryMessage,
  authCookieStorageMessage,
} = await import('@/lib/auth-cookie')
const { useAuthStore } = await import('@/stores/auth-store')
const { TELEGRAM_BIND_RESULT_MESSAGE } =
  await import('@/features/auth/constants')
const { TelegramBindDialog } = await import('./telegram-bind-dialog')

const i18n = createInstance()
await i18n.use(initReactI18next).init({
  lng: 'en',
  resources: { en: { translation: {} } },
})
await globalI18n.init({
  lng: 'en',
  resources: {
    en: {
      translation: {
        [authCookieRecoveryMessage]: authCookieRecoveryMessage,
        [authCookieStorageMessage]: authCookieStorageMessage,
      },
    },
  },
})
const originalAuth = useAuthStore.getState().auth
const originalAdapter = api.defaults.adapter
const originalSuccess = toast.success
const originalError = toast.error
const originalNow = Date.now
const successToasts: string[] = []
const errorToasts: string[] = []
const mounted: Array<{
  root: ReturnType<typeof createRoot>
  container: HTMLDivElement
}> = []

function owner(id = 7, sid = 'session-a') {
  useAuthStore.getState().auth.setBundle({
    access_token: `synthetic-${sid}`,
    token_type: 'Bearer',
    access_expires_at: Math.floor(Date.now() / 1000) + 600,
    user: { id, username: 'synthetic', role: 1 },
    session: {
      sid,
      current: true,
      login_method: 'synthetic',
      ip: '',
      user_agent: '',
      created_at: 1,
      last_active_at: 1,
      expires_at: 4_000_000_000,
    },
  })
}

function flow(
  character = 'a',
  expiresAt = Math.floor(Date.now() / 1000) + 300
) {
  const token = `lmmt_${character.repeat(64)}`
  return {
    flow_token: token,
    callback_url: `https://console.example.test/api/oauth/telegram/bind/${token}`,
    expires_at: expiresAt,
  }
}

function response(config: InternalAxiosRequestConfig, data: unknown) {
  return { config, data, status: 200, statusText: 'OK', headers: {} }
}

function deferred<T>() {
  let finish: ((value: T) => void) | undefined
  const promise = new Promise<T>((resolve) => {
    finish = resolve
  })
  return {
    promise,
    resolve(value: T) {
      assert.ok(finish)
      finish(value)
    },
  }
}

async function flush() {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 10))
  })
}

async function waitFor(predicate: () => boolean, message: string) {
  for (let attempt = 0; attempt < 100; attempt++) {
    await flush()
    if (predicate()) return
  }
  assert.fail(message)
}

async function renderDialog(
  onSuccess: () => void | Promise<void> = () => undefined
) {
  let changeOpen: ((value: boolean) => void) | undefined
  const changes: boolean[] = []
  function Harness() {
    const [open, setOpen] = useState(true)
    changeOpen = setOpen
    return (
      <I18nextProvider i18n={i18n}>
        <TelegramBindDialog
          open={open}
          botName='synthetic_bot'
          onOpenChange={(next) => {
            changes.push(next)
            setOpen(next)
          }}
          onSuccess={onSuccess}
        />
      </I18nextProvider>
    )
  }
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  mounted.push({ root, container })
  await act(async () => root.render(<Harness />))
  return {
    changes,
    async setOpen(value: boolean) {
      assert.ok(changeOpen)
      await act(async () => changeOpen?.(value))
    },
  }
}

function widget() {
  return document.querySelector<HTMLScriptElement>(
    'script[data-telegram-login]'
  )
}

function result(data: unknown, origin = window.location.origin) {
  return act(async () => {
    window.dispatchEvent(new MessageEvent('message', { origin, data }))
  })
}

function success(token = flow().flow_token) {
  return {
    type: TELEGRAM_BIND_RESULT_MESSAGE,
    flow_token: token,
    success: true,
  }
}

function retryButton() {
  const button = [...document.querySelectorAll('button')].find(
    (item) => item.textContent?.trim() === 'Retry'
  )
  assert.ok(button)
  return button
}

beforeEach(() => {
  owner()
  successToasts.length = 0
  errorToasts.length = 0
  toast.success = (message) => {
    successToasts.push(String(message))
    return 'synthetic-success'
  }
  toast.error = (message) => {
    errorToasts.push(String(message))
    return 'synthetic-error'
  }
})

afterEach(async () => {
  Date.now = originalNow
  for (const item of mounted.splice(0)) {
    await act(async () => item.root.unmount())
    item.container.remove()
  }
  api.defaults.adapter = originalAdapter
  useAuthStore.setState({ auth: originalAuth })
  toast.success = originalSuccess
  toast.error = originalError
  document.body.replaceChildren()
})

after(() => {
  domWindow.close()
  for (const [key, descriptor] of descriptors) {
    if (descriptor) Object.defineProperty(globalThis, key, descriptor)
    else Reflect.deleteProperty(globalThis, key)
  }
})

test('the Widget receives only the validated same-origin callback URL', async () => {
  api.defaults.adapter = async (config) => {
    assert.equal(config.url, '/api/oauth/telegram/bind/start')
    return response(config, { success: true, data: flow() })
  }
  await renderDialog()
  await waitFor(() => Boolean(widget()), 'Widget missing')
  assert.equal(widget()?.src, 'https://telegram.org/js/telegram-widget.js?22')
  assert.equal(widget()?.getAttribute('data-telegram-login'), 'synthetic_bot')
  assert.equal(widget()?.getAttribute('data-auth-url'), flow().callback_url)
  assert.equal(widget()?.hasAttribute('data-onauth'), false)
  assert.doesNotMatch(
    widget()?.outerHTML ?? '',
    /Bearer|synthetic-session|[?&]state=/
  )
})

test('unknown, malformed, wrong-flow and cross-origin messages cannot finish binding', async () => {
  let selfReads = 0
  let completions = 0
  api.defaults.adapter = async (config) => {
    if (config.url === '/api/user/self') selfReads++
    return response(config, { success: true, data: flow() })
  }
  const rendered = await renderDialog(() => {
    completions++
  })
  await waitFor(() => Boolean(widget()), 'Widget missing')
  for (const data of [
    null,
    [],
    { ...success(), success: 'true' },
    { ...success(), success: 1 },
    { type: TELEGRAM_BIND_RESULT_MESSAGE, flow_token: flow().flow_token },
    { ...success(), flow_token: 'unvalidated-flow' },
    success(flow('b').flow_token),
  ]) {
    await result(data)
  }
  await result(success(), 'https://elsewhere.example.test')
  await flush()
  assert.equal(selfReads, 0)
  assert.equal(completions, 0)
  assert.deepEqual(rendered.changes, [])
  assert.deepEqual(successToasts, [])
  assert.deepEqual(errorToasts, [])
})

test('success reads the actual binding and awaits profile refresh before closing', async () => {
  const refreshed = deferred<void>()
  const events: string[] = []
  let selfReads = 0
  api.defaults.adapter = async (config) => {
    if (config.url === '/api/user/self') {
      selfReads++
      events.push('actual-self')
      assert.equal(config.skipBusinessError, true)
      assert.equal(config.skipErrorHandler, true)
      return response(config, {
        success: true,
        data: { id: 7, telegram_id: '123456' },
      })
    }
    return response(config, { success: true, data: flow() })
  }
  const rendered = await renderDialog(async () => {
    events.push('refresh-start')
    await refreshed.promise
    events.push('refresh-finished')
  })
  await waitFor(() => Boolean(widget()), 'Widget missing')
  await result(success())
  await result(success())
  await waitFor(
    () => events.includes('refresh-start'),
    'profile refresh did not start'
  )
  assert.deepEqual(events, ['actual-self', 'refresh-start'])
  assert.equal(selfReads, 1)
  assert.match(document.body.textContent ?? '', /Refreshing\.\.\./)
  assert.deepEqual(rendered.changes, [])
  assert.deepEqual(successToasts, [])
  await act(async () => refreshed.resolve())
  await waitFor(
    () => rendered.changes.includes(false),
    'dialog did not close after refresh'
  )
  assert.deepEqual(events, ['actual-self', 'refresh-start', 'refresh-finished'])
  assert.deepEqual(successToasts, ['Binding successful!'])
  assert.equal(useAuthStore.getState().auth.user?.telegram_id, undefined)
})

for (const [name, body] of [
  ['missing binding', { success: true, data: { id: 7 } }],
  ['empty binding', { success: true, data: { id: 7, telegram_id: '' } }],
  ['zero binding', { success: true, data: { id: 7, telegram_id: '0' } }],
  [
    'garbage binding',
    { success: true, data: { id: 7, telegram_id: 'garbage' } },
  ],
  [
    'leading-zero binding',
    { success: true, data: { id: 7, telegram_id: '0123' } },
  ],
  [
    'whitespace binding',
    { success: true, data: { id: 7, telegram_id: ' 123 ' } },
  ],
  [
    'overflowing binding',
    { success: true, data: { id: 7, telegram_id: '9223372036854775808' } },
  ],
  ['negative binding', { success: true, data: { id: 7, telegram_id: '-1' } }],
  [
    'plus-prefixed binding',
    { success: true, data: { id: 7, telegram_id: '+123' } },
  ],
  ['other account', { success: true, data: { id: 8, telegram_id: '123456' } }],
  ['failed self response', { success: false, message: 'PRIVATE_SELF_DETAIL' }],
] as const) {
  test(`a callback does not claim success for ${name}`, async () => {
    let completions = 0
    api.defaults.adapter = async (config) =>
      response(
        config,
        config.url === '/api/user/self' ? body : { success: true, data: flow() }
      )
    const rendered = await renderDialog(() => {
      completions++
    })
    await waitFor(() => Boolean(widget()), 'Widget missing')
    await result(success())
    await waitFor(
      () => Boolean(document.querySelector('[role="alert"]')),
      'profile error missing'
    )
    assert.match(document.body.textContent ?? '', /Failed to load profile/)
    assert.doesNotMatch(document.body.textContent ?? '', /PRIVATE_SELF_DETAIL/)
    assert.equal(completions, 0)
    assert.deepEqual(rendered.changes, [])
    assert.deepEqual(successToasts, [])
    assert.equal(useAuthStore.getState().auth.user?.telegram_id, undefined)
  })
}

test('the maximum canonical positive i64 Telegram binding is accepted', async () => {
  let completions = 0
  api.defaults.adapter = async (config) =>
    response(
      config,
      config.url === '/api/user/self'
        ? { success: true, data: { id: 7, telegram_id: '9223372036854775807' } }
        : { success: true, data: flow() }
    )
  const rendered = await renderDialog(() => {
    completions++
  })
  await waitFor(() => Boolean(widget()), 'Widget missing')
  await result(success())
  await waitFor(
    () => rendered.changes.includes(false),
    'valid maximum binding did not close'
  )
  assert.equal(completions, 1)
  assert.deepEqual(successToasts, ['Binding successful!'])
  assert.equal(useAuthStore.getState().auth.user?.telegram_id, undefined)
})

for (const reason of ['uncertain', 'storage_unavailable'] as const) {
  for (const phase of ['start', 'actual-self', 'onSuccess'] as const) {
    test(`${phase} preserves the local ${reason} Cookie recovery instructions`, async () => {
      const blocked = new AuthCookieBlockedError(reason)
      const expected =
        reason === 'uncertain'
          ? authCookieRecoveryMessage
          : authCookieStorageMessage
      assert.equal(blocked.message, expected)
      let completions = 0
      api.defaults.adapter = async (config) => {
        if (
          phase === 'start' ||
          (phase === 'actual-self' && config.url === '/api/user/self')
        ) {
          throw Object.assign(blocked, { config })
        }
        return response(
          config,
          config.url === '/api/user/self'
            ? { success: true, data: { id: 7, telegram_id: '123456' } }
            : { success: true, data: flow() }
        )
      }
      const rendered = await renderDialog(async () => {
        if (phase === 'onSuccess') throw blocked
        completions++
      })
      if (phase !== 'start') {
        await waitFor(() => Boolean(widget()), 'Widget missing')
        await result(success())
      }
      await waitFor(
        () => Boolean(document.querySelector('p[role="alert"]')),
        'recovery instructions missing'
      )
      assert.equal(
        document.querySelector('p[role="alert"]')?.textContent,
        expected
      )
      assert.equal(widget(), null)
      assert.equal(completions, 0)
      assert.deepEqual(successToasts, [])
      assert.deepEqual(errorToasts, [])
      assert.deepEqual(rendered.changes, [])
    })
  }
}

test('a nonlocal error cannot impersonate the Cookie recovery class', async () => {
  api.defaults.adapter = async (config) =>
    response(
      config,
      config.url === '/api/user/self'
        ? { success: true, data: { id: 7, telegram_id: '123456' } }
        : { success: true, data: flow() }
    )
  const rendered = await renderDialog(async () => {
    throw Object.assign(new Error('PRIVATE_FAKE_COOKIE_GUIDANCE'), {
      name: 'AuthCookieBlockedError',
      code: 'AUTH_COOKIE_BLOCKED',
    })
  })
  await waitFor(() => Boolean(widget()), 'Widget missing')
  await result(success())
  await waitFor(
    () => Boolean(document.querySelector('p[role="alert"]')),
    'refresh error missing'
  )
  assert.equal(
    document.querySelector('p[role="alert"]')?.textContent,
    'Failed to load profile'
  )
  assert.doesNotMatch(
    document.body.textContent ?? '',
    /PRIVATE_FAKE_COOKIE_GUIDANCE/
  )
  assert.equal(widget(), null)
  assert.deepEqual(successToasts, [])
  assert.deepEqual(rendered.changes, [])
})

for (const [name, payload, expected] of [
  [
    'unknown error',
    {
      success: false,
      code: 'PRIVATE_UNRECOGNIZED_CODE',
      message: 'PRIVATE_BACKEND_DETAIL',
    },
    'Failed to start Telegram binding',
  ],
  [
    'known disabled account',
    {
      success: false,
      code: 'AUTH_USER_DISABLED',
      message: 'PRIVATE_BACKEND_DETAIL',
    },
    'This user account is disabled.',
  ],
  [
    'malformed success',
    { success: 'true', data: flow(), message: 'PRIVATE_BACKEND_DETAIL' },
    'Failed to start Telegram binding',
  ],
] as const) {
  test(`${name} is shown without raw provider text`, async () => {
    api.defaults.adapter = async (config) => response(config, payload)
    await renderDialog()
    await waitFor(
      () => Boolean(document.querySelector('[role="alert"]')),
      'start error missing'
    )
    assert.match(
      document.body.textContent ?? '',
      new RegExp(expected.replaceAll('.', '\\.'))
    )
    assert.doesNotMatch(
      document.body.textContent ?? '',
      /PRIVATE_|appeal form|\.callback_url/
    )
    assert.deepEqual(errorToasts, [])
    assert.equal(widget(), null)
  })
}

test('an HTTP start error is local and does not leak its server message', async () => {
  api.defaults.adapter = async (config) => {
    throw new AxiosError('PRIVATE_HTTP_DETAIL', undefined, config, undefined, {
      config,
      data: { message: 'PRIVATE_SERVER_MESSAGE' },
      status: 503,
      statusText: 'Unavailable',
      headers: {},
    })
  }
  await renderDialog()
  await waitFor(
    () => Boolean(document.querySelector('[role="alert"]')),
    'HTTP error missing'
  )
  assert.match(
    document.body.textContent ?? '',
    /Failed to start Telegram binding/
  )
  assert.doesNotMatch(document.body.textContent ?? '', /PRIVATE_/)
  assert.deepEqual(errorToasts, [])
})

test('a failed result is consumed and retry rejects messages from its old flow', async () => {
  let starts = 0
  let selfReads = 0
  api.defaults.adapter = async (config) => {
    if (config.url === '/api/user/self') {
      selfReads++
      return response(config, {
        success: true,
        data: { id: 7, telegram_id: '123456' },
      })
    }
    starts++
    return response(config, {
      success: true,
      data: flow(starts === 1 ? 'a' : 'b'),
    })
  }
  await renderDialog()
  await waitFor(() => Boolean(widget()), 'Widget missing')
  await result({ ...success(), success: false, code: 'PRIVATE_UNKNOWN_RESULT' })
  assert.match(
    document.body.textContent ?? '',
    /Telegram binding failed\. Please try again\./
  )
  assert.doesNotMatch(document.body.textContent ?? '', /PRIVATE_UNKNOWN_RESULT/)
  await act(async () => retryButton().click())
  await waitFor(
    () => widget()?.getAttribute('data-auth-url') === flow('b').callback_url,
    'replacement Widget missing'
  )
  await result(success())
  assert.equal(selfReads, 0)
  await result(success(flow('b').flow_token))
  await waitFor(() => selfReads === 1, 'current flow did not read self')
})

test('closing invalidates a delayed start and reopening uses a new flow', async () => {
  const late = deferred<unknown>()
  let starts = 0
  api.defaults.adapter = async (config) => {
    starts++
    return response(
      config,
      starts === 1 ? await late.promise : { success: true, data: flow('b') }
    )
  }
  const rendered = await renderDialog()
  await waitFor(() => starts === 1, 'start missing')
  await rendered.setOpen(false)
  await rendered.setOpen(true)
  await waitFor(
    () => widget()?.getAttribute('data-auth-url') === flow('b').callback_url,
    'reopened flow missing'
  )
  await act(async () => late.resolve({ success: true, data: flow('a') }))
  await flush()
  assert.equal(widget()?.getAttribute('data-auth-url'), flow('b').callback_url)
  await result(success())
  assert.deepEqual(successToasts, [])
})

for (const [name, nextId, nextSid] of [
  ['account switch', 8, 'session-b'],
  ['same-account session switch', 7, 'session-b'],
] as const) {
  test(`${name} invalidates delayed starts and old result messages`, async () => {
    const late = deferred<unknown>()
    let starts = 0
    let selfReads = 0
    api.defaults.adapter = async (config) => {
      if (config.url === '/api/user/self') selfReads++
      starts++
      return response(
        config,
        starts === 1 ? await late.promise : { success: true, data: flow('b') }
      )
    }
    const rendered = await renderDialog()
    await waitFor(() => starts === 1, 'first start missing')
    await act(async () => owner(nextId, nextSid))
    await waitFor(
      () => widget()?.getAttribute('data-auth-url') === flow('b').callback_url,
      'new identity flow missing'
    )
    await act(async () => late.resolve({ success: true, data: flow('a') }))
    await result(success())
    assert.equal(
      widget()?.getAttribute('data-auth-url'),
      flow('b').callback_url
    )
    assert.equal(selfReads, 0)
    assert.deepEqual(rendered.changes, [])
    assert.deepEqual(successToasts, [])
  })
}

test('closing during actual-self refresh invalidates its late success', async () => {
  const lateSelf = deferred<unknown>()
  let selfReads = 0
  let completions = 0
  api.defaults.adapter = async (config) => {
    if (config.url === '/api/user/self') {
      selfReads++
      return response(config, await lateSelf.promise)
    }
    return response(config, { success: true, data: flow() })
  }
  const rendered = await renderDialog(() => {
    completions++
  })
  await waitFor(() => Boolean(widget()), 'Widget missing')
  await result(success())
  await waitFor(() => selfReads === 1, 'actual self missing')
  await rendered.setOpen(false)
  await act(async () =>
    lateSelf.resolve({ success: true, data: { id: 7, telegram_id: '123456' } })
  )
  await flush()
  assert.equal(completions, 0)
  assert.deepEqual(successToasts, [])
})

test('expired flow results cannot trigger a profile read or success', async () => {
  const now = originalNow()
  const expiring = flow('a', Math.floor(now / 1000) + 2)
  let selfReads = 0
  api.defaults.adapter = async (config) => {
    if (config.url === '/api/user/self') selfReads++
    return response(config, { success: true, data: expiring })
  }
  const rendered = await renderDialog()
  await waitFor(() => Boolean(widget()), 'Widget missing')
  Date.now = () => now + 3_000
  await result(success())
  assert.equal(selfReads, 0)
  assert.match(
    document.body.textContent ?? '',
    /This Telegram binding request has expired or has already been used\./
  )
  assert.deepEqual(successToasts, [])
  assert.deepEqual(rendered.changes, [])
})
