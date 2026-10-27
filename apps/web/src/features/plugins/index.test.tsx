/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, afterEach, test } from 'node:test'

import { AxiosError } from 'axios'
import { Window } from 'happy-dom'

const dom = new Window()
for (const name of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'Element',
  'Node',
  'Event',
  'MouseEvent',
] as const) {
  Object.defineProperty(globalThis, name, {
    configurable: true,
    value: name === 'window' ? dom : dom[name],
  })
}
Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
  configurable: true,
  value: true,
})
const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { QueryClient, QueryClientProvider } =
  await import('@tanstack/react-query')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { api } = await import('@/lib/api')
const { useAuthStore } = await import('@/stores/auth-store')
const { PluginsPage } = await import('./index')
const previousAuth = useAuthStore.getState().auth
const originalAdapter = api.defaults.adapter
const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: {} } } })
let dispose: (() => Promise<void>) | undefined

function login(role: number) {
  useAuthStore.getState().auth.setBundle({
    access_token: 'synthetic-credential',
    token_type: 'Bearer',
    access_expires_at: 2000000000,
    user: { id: 901, role, username: 'synthetic-user' },
    session: {
      sid: 'synthetic-session',
      current: true,
      login_method: 'password',
      ip: '127.0.0.1',
      user_agent: 'synthetic',
      created_at: 1,
      last_active_at: 1,
      expires_at: 2000000000,
    },
  })
}

async function mount() {
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  await act(async () =>
    root.render(
      <QueryClientProvider client={client}>
        <I18nextProvider i18n={i18n}>
          <PluginsPage />
        </I18nextProvider>
      </QueryClientProvider>
    )
  )
  dispose = async () => {
    await act(async () => root.unmount())
    client.clear()
  }
  return container
}

async function until(check: () => boolean) {
  for (let attempt = 0; attempt < 100; attempt++) {
    if (check()) return
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 10))
    })
  }
  assert.ok(check(), 'Plugin page did not reach the expected state')
}

afterEach(async () => {
  await dispose?.()
  dispose = undefined
  api.defaults.adapter = originalAdapter
  useAuthStore.setState({ auth: previousAuth })
  document.body.replaceChildren()
})
after(() => dom.close())

test('administrator sees plugin state and refreshes with a scoped read only request', async () => {
  login(100)
  let calls = 0
  api.defaults.adapter = async (config) => {
    assert.equal(config.url, '/api/extensions/plugins')
    assert.equal(config.method, 'get')
    assert.equal(
      config.headers.get('Authorization'),
      'Bearer synthetic-credential'
    )
    assert.deepEqual(config.authScope, {
      userId: 901,
      sessionId: 'synthetic-session',
    })
    calls++
    return {
      config,
      headers: {},
      status: 200,
      statusText: 'OK',
      data: {
        success: true,
        data: [
          {
            name: 'store',
            version: '1.0.0',
            api_version: 1,
            registration_sha256: 'a'.repeat(64),
            available: true,
            installed: true,
            enabled: calls === 1,
          },
        ],
      },
    }
  }
  const container = await mount()
  await until(() => container.textContent?.includes('Enabled modules') === true)
  assert.match(container.textContent ?? '', /store/)
  assert.match(container.textContent ?? '', /Installed/)
  assert.match(
    container.textContent ?? '',
    /Registry entries are declarations, not service health checks/
  )
  assert.deepEqual(
    [...container.querySelectorAll('button')].map(
      (button) => button.textContent
    ),
    ['Refresh']
  )
  await act(async () => container.querySelector('button')?.click())
  await until(
    () => calls === 2 && container.textContent?.includes('Disabled') === true
  )
  assert.match(container.textContent ?? '', /No data available/)
})

test('non-admin cannot query, and invalid API responses render an error', async () => {
  let calls = 0
  api.defaults.adapter = async (config) => {
    calls++
    return {
      config,
      headers: {},
      status: 200,
      statusText: 'OK',
      data: '<html>not the API</html>',
    }
  }
  login(1)
  let container = await mount()
  assert.match(container.textContent ?? '', /administrator session is required/)
  assert.equal(calls, 0)
  await dispose?.()
  dispose = undefined
  document.body.replaceChildren()
  login(100)
  container = await mount()
  await until(
    () => container.textContent?.includes('Oops! Something went wrong') === true
  )
  assert.equal(calls, 1)
  assert.match(
    container.textContent ?? '',
    /The plugin response does not match this release/
  )
  assert.doesNotMatch(container.textContent ?? '', /No data available/)
})

test('failed refresh retains the last registry as stale and distinguishes an upstream 503', async () => {
  login(100)
  let calls = 0
  api.defaults.adapter = async (config) => {
    if (++calls > 1) {
      throw new AxiosError(
        'unavailable',
        'ERR_BAD_RESPONSE',
        config,
        undefined,
        {
          config,
          headers: {},
          status: 503,
          statusText: 'Unavailable',
          data: { success: false, message: 'synthetic outage' },
        }
      )
    }
    return {
      config,
      headers: {},
      status: 200,
      statusText: 'OK',
      data: {
        success: true,
        data: [
          {
            name: 'toolmarket',
            version: '1.0.0',
            api_version: 1,
            registration_sha256: 'b'.repeat(64),
            available: true,
            installed: true,
            enabled: true,
          },
        ],
      },
    }
  }
  const container = await mount()
  await until(() => container.textContent?.includes('toolmarket') === true)
  assert.match(container.textContent ?? '', /Registration fingerprint/)
  assert.match(container.textContent ?? '', /Route configuration/)
  await act(async () => container.querySelector('button')?.click())
  await until(
    () =>
      container.textContent?.includes('Plugin status is not current') === true
  )
  assert.match(container.textContent ?? '', /toolmarket/)
  assert.match(
    container.textContent ?? '',
    /extension service is unavailable \(503\)/
  )
  assert.equal(calls, 2)
})
