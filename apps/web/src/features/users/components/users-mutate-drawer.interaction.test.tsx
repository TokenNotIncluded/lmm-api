/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, afterEach, test } from 'node:test'

import { Window } from 'happy-dom'

import type { PermissionCatalog } from '@/lib/admin-permissions'

import type { User } from '../types'

const domWindow = new Window({
  url: 'https://console.example.test/admin/users',
})
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'HTMLButtonElement',
  'HTMLInputElement',
  'HTMLTextAreaElement',
  'HTMLLabelElement',
  'SVGElement',
  'Node',
  'Element',
  'Event',
  'MouseEvent',
  'PointerEvent',
  'FocusEvent',
  'CustomEvent',
  'MutationObserver',
  'ResizeObserver',
] as const) {
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: domWindow[key],
  })
}
Object.defineProperties(globalThis, {
  requestAnimationFrame: {
    configurable: true,
    value: (callback: FrameRequestCallback) => setTimeout(() => callback(0), 0),
  },
  cancelAnimationFrame: {
    configurable: true,
    value: (handle: number) => clearTimeout(handle),
  },
  getComputedStyle: {
    configurable: true,
    value: domWindow.getComputedStyle.bind(domWindow),
  },
  IS_REACT_ACT_ENVIRONMENT: { configurable: true, value: true },
})

const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { QueryClient, QueryClientProvider } =
  await import('@tanstack/react-query')
const { api } = await import('@/lib/api')
const { useAuthStore } = await import('@/stores/auth-store')
const { createQueryRetry } = await import('@/lib/query-retry')
const { UsersProvider } = await import('./users-provider')
const { UsersMutateDrawer } = await import('./users-mutate-drawer')
const originalGet = api.get
const originalPut = api.put

const i18n = createInstance()
await i18n.use(initReactI18next).init({
  lng: 'en',
  resources: { en: { translation: {} } },
})

const catalog: PermissionCatalog = {
  resources: [
    {
      resource: 'channel',
      label_key: 'Channels',
      actions: [
        { action: 'read', label_key: 'Read channels', description_key: '' },
        {
          action: 'secret_view',
          label_key: 'View secrets',
          description_key: '',
        },
      ],
    },
  ],
  roles: [
    {
      key: 'admin',
      name: 'Administrator',
      built_in: true,
      superuser: false,
      grants: { channel: { read: true, secret_view: false } },
    },
  ],
}
const user: User = {
  id: 41,
  username: 'synthetic-admin',
  display_name: 'Synthetic administrator',
  role: 10,
  status: 1,
  quota: 0,
  used_quota: 0,
  request_count: 0,
  group: 'default',
  admin_permissions: { channel: { read: false, secret_view: false } },
}
const unavailableMessage =
  'Administrator permissions could not be loaded. Retry before saving.'

async function flushEffects() {
  await new Promise((resolve) => setTimeout(resolve, 20))
}

async function waitFor(condition: () => boolean) {
  for (let attempt = 0; attempt < 40 && !condition(); attempt += 1) {
    await act(flushEffects)
  }
  assert.ok(condition())
}

function button(text: string) {
  const found = [
    ...document.querySelectorAll<HTMLButtonElement>('button'),
  ].find((candidate) => candidate.textContent === text)
  assert.ok(found, `Missing button: ${text}`)
  return found
}

async function renderDrawer(actorRole = 100, cachedCatalog = false) {
  useAuthStore
    .getState()
    .auth.setUser({ id: 99, username: 'synthetic-actor', role: actorRole })
  const client = new QueryClient({
    defaultOptions: {
      queries: { retry: createQueryRetry(true), retryDelay: 0 },
    },
  })
  if (cachedCatalog) {
    client.setQueryData(
      [
        'admin-permission-catalog',
        99,
        actorRole,
        useAuthStore.getState().auth.session?.sid,
      ],
      catalog
    )
  }
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  const render = async (row: User) =>
    act(async () => {
      root.render(
        <QueryClientProvider client={client}>
          <I18nextProvider i18n={i18n}>
            <UsersProvider>
              <UsersMutateDrawer
                open
                onOpenChange={() => {}}
                currentRow={row}
              />
            </UsersProvider>
          </I18nextProvider>
        </QueryClientProvider>
      )
      await flushEffects()
    })
  await render(user)
  const cleanup = async () => {
    await act(async () => root.unmount())
    client.clear()
  }
  return Object.assign(cleanup, { rerender: render })
}

afterEach(() => {
  api.get = originalGet
  api.put = originalPut
  useAuthStore.getState().auth.setUser(null)
  document.body.replaceChildren()
})
after(() => domWindow.close())

for (const failure of [
  'business',
  'malformed',
  'forbidden',
  'unavailable',
] as const) {
  test(`${failure} permission reads block writes and recover after explicit retry`, async () => {
    let available = false
    let catalogReads = 0
    const writes: unknown[] = []
    api.get = (async (url: string) => {
      if (url === '/api/authz/catalog') {
        catalogReads += 1
        if (!available) {
          if (failure === 'forbidden' || failure === 'unavailable') {
            throw new Error(failure === 'forbidden' ? 'HTTP 403' : 'HTTP 503')
          }
          return {
            data:
              failure === 'business'
                ? { success: false }
                : { success: true, data: { resources: [], roles: [] } },
          }
        }
        return { data: { success: true, data: catalog } }
      }
      if (url === '/api/user/41') return { data: { success: true, data: user } }
      if (url === '/api/group/') {
        return { data: { success: true, data: ['default'] } }
      }
      if (url === '/api/user/41/announcements') {
        return { data: { success: true, data: [] } }
      }
      return { data: { success: false } }
    }) as typeof api.get
    api.put = (async (url: string, data: unknown) => {
      assert.equal(url, '/api/user/')
      writes.push(data)
      return { data: { success: true } }
    }) as typeof api.put

    // A failed fresh read also invalidates a previously successful cache entry.
    const cleanup = await renderDrawer(100, true)
    try {
      await waitFor(
        () => document.body.textContent?.includes(unavailableMessage) === true
      )
      assert.equal(button('Save changes').disabled, true)
      assert.equal(document.querySelectorAll('[role="checkbox"]').length, 0)
      await act(async () => {
        document
          .querySelector('form')
          ?.dispatchEvent(
            new Event('submit', { bubbles: true, cancelable: true })
          )
        await flushEffects()
      })
      assert.equal(writes.length, 0)

      available = true
      await act(async () => {
        button('Retry').click()
        await flushEffects()
      })
      await waitFor(() => !button('Save changes').disabled)
      assert.equal(catalogReads, 2)
      assert.equal(
        document.body.textContent?.includes(unavailableMessage),
        false
      )
      assert.equal(document.querySelectorAll('[role="checkbox"]').length, 2)
      await act(async () => {
        button('Save changes').click()
        await flushEffects()
      })
      assert.equal(writes.length, 1)
      assert.deepEqual(
        (writes[0] as { admin_permissions: unknown }).admin_permissions,
        user.admin_permissions
      )
    } finally {
      await cleanup()
    }
  })
}

test('an actor without permission editing rights does not read or echo permission matrices', async () => {
  const writes: Array<Record<string, unknown>> = []
  api.get = (async (url: string) => {
    assert.notEqual(url, '/api/authz/catalog')
    if (url === '/api/user/41') return { data: { success: true, data: user } }
    if (url === '/api/group/') {
      return { data: { success: true, data: ['default'] } }
    }
    if (url === '/api/user/41/announcements') {
      return { data: { success: true, data: [] } }
    }
    return { data: { success: false } }
  }) as typeof api.get
  api.put = (async (_url: string, data: Record<string, unknown>) => {
    writes.push(data)
    return { data: { success: true } }
  }) as typeof api.put
  const cleanup = await renderDrawer(10, true)
  try {
    await waitFor(
      () =>
        document.querySelector<HTMLInputElement>('input')?.value ===
        user.username
    )
    assert.equal(button('Save changes').disabled, false)
    await act(async () => {
      button('Save changes').click()
      await flushEffects()
    })
    assert.equal(writes.length, 1)
    assert.equal(Object.hasOwn(writes[0], 'admin_permissions'), false)
  } finally {
    await cleanup()
  }
})

for (const failure of [
  'business',
  'transport',
  'wrong-target',
  'missing-permissions',
] as const) {
  test(`target change clears the old form and blocks ${failure} reads until explicit recovery`, async () => {
    const other: User = {
      ...user,
      id: 42,
      username: 'synthetic-admin-b',
      display_name: 'Administrator B',
      admin_permissions: { channel: { read: true, secret_view: false } },
    }
    let available = false
    const writes: Array<Record<string, unknown>> = []
    api.get = (async (url: string) => {
      if (url === '/api/authz/catalog') {
        return { data: { success: true, data: catalog } }
      }
      if (url === '/api/user/41') return { data: { success: true, data: user } }
      if (url === '/api/user/42') {
        if (!available) {
          if (failure === 'transport') throw new Error('HTTP 503')
          if (failure === 'business') return { data: { success: false } }
          if (failure === 'wrong-target') {
            return { data: { success: true, data: user } }
          }
          const { admin_permissions: _permissions, ...incomplete } = other
          return { data: { success: true, data: incomplete } }
        }
        return { data: { success: true, data: other } }
      }
      if (url === '/api/group/') {
        return { data: { success: true, data: ['default'] } }
      }
      return { data: { success: false } }
    }) as typeof api.get
    api.put = (async (_url: string, data: Record<string, unknown>) => {
      writes.push(data)
      return { data: { success: true } }
    }) as typeof api.put
    const cleanup = await renderDrawer()
    try {
      await waitFor(() => !button('Save changes').disabled)
      assert.equal(
        document.querySelector<HTMLInputElement>('input')?.value,
        user.username
      )
      await cleanup.rerender(other)
      await waitFor(
        () =>
          document.body.textContent?.includes('Failed to load profile') === true
      )
      assert.equal(button('Save changes').disabled, true)
      assert.notEqual(
        document.querySelector<HTMLInputElement>('input')?.value,
        user.username
      )
      await act(async () => {
        document
          .querySelector('form')
          ?.dispatchEvent(
            new Event('submit', { bubbles: true, cancelable: true })
          )
        await flushEffects()
      })
      assert.equal(writes.length, 0)
      available = true
      await act(async () => {
        button('Retry').click()
        await flushEffects()
      })
      await waitFor(() => !button('Save changes').disabled)
      await act(async () => {
        button('Save changes').click()
        await flushEffects()
      })
      assert.equal(writes.length, 1)
      assert.equal(writes[0].id, other.id)
      assert.equal(writes[0].username, other.username)
      assert.deepEqual(writes[0].admin_permissions, other.admin_permissions)
    } finally {
      await cleanup()
    }
  })
}

test('a late previous-target read cannot overwrite the current target or authorize its write', async () => {
  const other: User = { ...user, id: 42, username: 'synthetic-admin-b' }
  let resolvePrevious: ((value: unknown) => void) | undefined
  const writes: Array<Record<string, unknown>> = []
  api.get = (async (url: string) => {
    if (url === '/api/authz/catalog') {
      return { data: { success: true, data: catalog } }
    }
    if (url === '/api/user/41') {
      return new Promise((resolve) => {
        resolvePrevious = resolve
      })
    }
    if (url === '/api/user/42') return { data: { success: true, data: other } }
    if (url === '/api/group/') {
      return { data: { success: true, data: ['default'] } }
    }
    return { data: { success: false } }
  }) as typeof api.get
  api.put = (async (_url: string, data: Record<string, unknown>) => {
    writes.push(data)
    return { data: { success: true } }
  }) as typeof api.put
  const cleanup = await renderDrawer()
  try {
    assert.equal(button('Save changes').disabled, true)
    await cleanup.rerender(other)
    await waitFor(() => !button('Save changes').disabled)
    assert.ok(resolvePrevious)
    await act(async () => {
      resolvePrevious?.({ data: { success: true, data: user } })
      await flushEffects()
    })
    assert.equal(
      document.querySelector<HTMLInputElement>('input')?.value,
      other.username
    )
    await act(async () => {
      button('Save changes').click()
      await flushEffects()
    })
    assert.equal(writes.length, 1)
    assert.equal(writes[0].id, other.id)
    assert.equal(writes[0].username, other.username)
  } finally {
    await cleanup()
  }
})

test('a new SID for the same actor requires a fresh successful target read', async () => {
  let unavailable = false
  const writes: unknown[] = []
  api.get = (async (url: string) => {
    if (url === '/api/authz/catalog') {
      return { data: { success: true, data: catalog } }
    }
    if (url === '/api/user/41') {
      if (unavailable) throw new Error('HTTP 503')
      return { data: { success: true, data: user } }
    }
    if (url === '/api/group/') {
      return { data: { success: true, data: ['default'] } }
    }
    return { data: { success: false } }
  }) as typeof api.get
  api.put = (async (_url: string, data: unknown) => {
    writes.push(data)
    return { data: { success: true } }
  }) as typeof api.put
  const cleanup = await renderDrawer()
  try {
    await waitFor(() => !button('Save changes').disabled)
    unavailable = true
    await act(async () => {
      useAuthStore.getState().auth.setBundle({
        access_token: 'synthetic-catalog-access',
        token_type: 'Bearer',
        access_expires_at: 2000000000,
        user: { id: 99, username: 'synthetic-actor', role: 100 },
        session: {
          sid: 'synthetic-new-sid',
          current: true,
          login_method: 'password',
          ip: '127.0.0.1',
          user_agent: 'synthetic',
          created_at: 1,
          last_active_at: 1,
          expires_at: 2000000000,
        },
      })
      await flushEffects()
    })
    await waitFor(
      () =>
        document.body.textContent?.includes('Failed to load profile') === true
    )
    assert.equal(document.querySelector<HTMLInputElement>('input')?.value, '')
    assert.equal(button('Save changes').disabled, true)
    await act(async () => {
      document
        .querySelector('form')
        ?.dispatchEvent(
          new Event('submit', { bubbles: true, cancelable: true })
        )
      await flushEffects()
    })
    assert.equal(writes.length, 0)
  } finally {
    await cleanup()
    useAuthStore.getState().auth.reset()
  }
})
