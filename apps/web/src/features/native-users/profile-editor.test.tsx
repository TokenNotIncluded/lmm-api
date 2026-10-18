/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, afterEach, test } from 'node:test'

import { Window } from 'happy-dom'

const dom = new Window({ url: 'https://console.example.test/users' })
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
    value: dom[key],
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
    value: dom.getComputedStyle.bind(dom),
  },
  IS_REACT_ACT_ENVIRONMENT: { configurable: true, value: true },
})
after(() => dom.close())

const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { QueryClient, QueryClientProvider } =
  await import('@tanstack/react-query')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { api } = await import('@/lib/api')
const { useAuthStore } = await import('@/stores/auth-store')
const { readNativeUsersScope } = await import('./access')
const { NativeDisplayNameEditor } = await import('./profile-editor')
const { NativeAdminUsersEntry } = await import('./entry')
const { NativeUsersPage } = await import('./page')
const { nativeUsersResponseSchema } = await import('./contract')
const { nativeListFixture, nativeUserFixture, nativeStatusFixture } =
  await import('./test-fixtures')
const { PROFILE_PROTOCOL } = await import('./profile-contract')
const originalGet = api.get
const originalPatch = api.patch
const cleanups: Array<() => void> = []
const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: {} } } })

const capabilities = {
  version: 1,
  contract: PROFILE_PROTOCOL,
  operations: { read: true, update_display_name: true },
  target_platform_roles: ['user'],
  display_name_limits: {
    max_code_points: 20,
    max_utf8_bytes: 80,
    controls_allowed: false,
  },
  preconditions: ['auth_version', 'display_name'],
}
const profile = {
  version: 1,
  contract: PROFILE_PROTOCOL,
  user_id: '9007199254740993',
  auth_version: '9007199254740995',
  display_name: 'fresh server name',
  platform_role: 'user',
  active: false,
  account_active: false,
}
const profilePath = `/core/v1/admin/user-profiles/${profile.user_id}`
const capabilityPath = '/core/v1/admin/user-profiles/capabilities'
const unknownMessage =
  'Save result is unknown. Check the original operation before continuing.'

function login(sid = 'synthetic-profile-session') {
  useAuthStore.getState().auth.setBundle({
    access_token: 'synthetic-profile-access',
    token_type: 'Bearer',
    access_expires_at: 2000000000,
    user: { id: 7, role: 10, username: 'synthetic-admin' },
    session: {
      sid,
      current: true,
      login_method: 'password',
      ip: '127.0.0.1',
      user_agent: 'synthetic',
      created_at: 1,
      last_active_at: 1,
      expires_at: 2000000000,
    },
  })
  const scope = readNativeUsersScope()
  assert.ok(scope)
  return scope
}
async function pause() {
  await new Promise((resolve) => setTimeout(resolve, 20))
}
async function waitFor(condition: () => boolean) {
  for (let index = 0; index < 60 && !condition(); index += 1) await act(pause)
  assert.ok(condition())
}
function button(label: string) {
  const element = [
    ...document.querySelectorAll<HTMLButtonElement>('button'),
  ].find((value) => value.textContent === label)
  assert.ok(element, `Missing button ${label}`)
  return element
}
function input() {
  const element = document.querySelector<HTMLInputElement>(
    `#native-display-name-${profile.user_id}`
  )
  assert.ok(element)
  return element
}
async function edit(value: string) {
  const setValue = Object.getOwnPropertyDescriptor(
    HTMLInputElement.prototype,
    'value'
  )?.set
  assert.ok(setValue)
  await act(async () => {
    setValue.call(input(), value)
    input().dispatchEvent(new Event('input', { bubbles: true }))
    input().dispatchEvent(new Event('change', { bubbles: true }))
    await pause()
  })
}
async function mount() {
  const scope = login()
  let updated = 0
  let denied = 0
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  cleanups.push(() => {
    root.unmount()
    container.remove()
  })
  await act(async () => {
    root.render(
      <I18nextProvider i18n={i18n}>
        <NativeDisplayNameEditor
          scope={scope}
          userId={profile.user_id}
          onUpdated={() => {
            updated += 1
          }}
          onAccessDenied={() => {
            denied += 1
          }}
        />
      </I18nextProvider>
    )
    await pause()
  })
  return { updated: () => updated, denied: () => denied }
}
async function openEditor() {
  await act(async () => {
    button('Edit · Display name').click()
    await pause()
  })
}
function allowReads(
  getProfile: () => typeof profile = () => profile,
  reads: string[] = []
) {
  api.get = (async (url: string) => {
    reads.push(url)
    assert.ok([capabilityPath, profilePath].includes(url))
    return { data: url === capabilityPath ? capabilities : getProfile() }
  }) as typeof api.get
}
afterEach(async () => {
  await act(async () => {
    for (const cleanup of cleanups.splice(0)) cleanup()
  })
  api.get = originalGet
  api.patch = originalPatch
  useAuthStore.getState().auth.reset('idle')
  document.body.replaceChildren()
})

test('opening requires the independent capability and a fresh profile before one conditional PATCH', async () => {
  const reads: string[] = []
  allowReads(() => profile, reads)
  const writes: unknown[] = []
  api.patch = (async (url: string, data: { display_name: string }) => {
    assert.equal(url, profilePath)
    writes.push(data)
    return {
      data: {
        version: 1,
        contract: PROFILE_PROTOCOL,
        changed: true,
        profile: { ...profile, display_name: data.display_name },
      },
    }
  }) as typeof api.patch
  const state = await mount()
  assert.deepEqual(reads, [])
  await openEditor()
  await waitFor(() => input().value === profile.display_name)
  assert.deepEqual(reads, [capabilityPath, profilePath])
  await edit('😀'.repeat(20))
  assert.equal(button('Save changes').disabled, false)
  await act(async () => {
    button('Save changes').click()
    await pause()
  })
  assert.deepEqual(writes, [
    {
      expected_auth_version: profile.auth_version,
      expected_display_name: profile.display_name,
      display_name: '😀'.repeat(20),
    },
  ])
  assert.equal(state.updated(), 1)
})

test('missing or false profile capabilities never reach target reads or writes', async () => {
  const reads: string[] = []
  api.get = (async (url: string) => {
    reads.push(url)
    return {
      data: {
        ...capabilities,
        operations: { read: true, update_display_name: false },
      },
    }
  }) as typeof api.get
  api.patch = (async () => {
    assert.fail('no PATCH permitted')
  }) as typeof api.patch
  await mount()
  await openEditor()
  await waitFor(
    () => document.body.textContent?.includes('Failed to load profile') === true
  )
  assert.deepEqual(reads, [capabilityPath])
  assert.equal(button('Save changes').disabled, true)
})

test('409 preserves the draft but requires a new GET and explicit second conditional save', async () => {
  let current = { ...profile }
  allowReads(() => current)
  const writes: Array<{ expected_display_name: string; display_name: string }> =
    []
  api.patch = (async (
    _url: string,
    body: { expected_display_name: string; display_name: string }
  ) => {
    writes.push(body)
    if (writes.length === 1) {
      current = { ...profile, display_name: 'concurrent change' }
      throw Object.assign(new Error('conflict'), { response: { status: 409 } })
    }
    return {
      data: {
        version: 1,
        contract: PROFILE_PROTOCOL,
        changed: true,
        profile: { ...current, display_name: body.display_name },
      },
    }
  }) as typeof api.patch
  const state = await mount()
  await openEditor()
  await waitFor(() => input().value === profile.display_name)
  await edit('desired name')
  await act(async () => {
    button('Save changes').click()
    await pause()
  })
  assert.equal(input().value, 'desired name')
  assert.equal(button('Save changes').disabled, true)
  assert.equal(writes.length, 1)
  await act(async () => {
    button('Reload').click()
    await pause()
  })
  await waitFor(
    () => document.body.textContent?.includes('concurrent change') === true
  )
  assert.equal(input().value, 'desired name')
  assert.equal(writes.length, 1, 'reload is read-only')
  await act(async () => {
    button('Save changes').click()
    await pause()
  })
  assert.equal(writes[1].expected_display_name, 'concurrent change')
  assert.equal(state.updated(), 1)
})

for (const failure of [503, 'transport'] as const) {
  test(`${failure} with a matching readback remains UNKNOWN instead of reporting this write successful`, async () => {
    let current = { ...profile }
    allowReads(() => current)
    let writes = 0
    api.patch = (async (_url: string, data: { display_name: string }) => {
      writes += 1
      current = { ...profile, display_name: data.display_name }
      throw Object.assign(new Error('unknown outcome'), {
        response: { status: failure === 503 ? 503 : undefined },
      })
    }) as typeof api.patch
    const state = await mount()
    await openEditor()
    await waitFor(() => input().value === profile.display_name)
    await edit('desired name')
    await act(async () => {
      button('Save changes').click()
      await pause()
    })
    await waitFor(
      () => document.body.textContent?.includes(unknownMessage) === true
    )
    assert.equal(button('Save changes').disabled, true)
    await act(async () => {
      button('Reload').click()
      await pause()
    })
    await waitFor(
      () =>
        document
          .querySelector('output')
          ?.textContent?.includes('desired name') === true
    )
    assert.equal(input().value, 'desired name')
    assert.equal(button('Save changes').disabled, true)
    assert.ok(document.body.textContent?.includes(unknownMessage))
    assert.equal(writes, 1)
    assert.equal(state.updated(), 0)
    assert.doesNotMatch(
      document.body.textContent ?? '',
      /changed successfully|No changes/
    )
  })
}

test('historical controls and markup are text, while invalid new names cannot dispatch', async () => {
  const historical = { ...profile, display_name: '<img src=x onerror=x>\n' }
  allowReads(() => historical)
  let writes = 0
  api.patch = (async () => {
    writes += 1
    assert.fail('invalid name must not write')
  }) as typeof api.patch
  await mount()
  await openEditor()
  await waitFor(
    () =>
      document.querySelector('output')?.textContent?.includes('<img') === true
  )
  assert.equal(document.querySelectorAll('img,script').length, 0)
  assert.equal(button('Save changes').disabled, true)
  await edit('x'.repeat(21))
  assert.equal(button('Save changes').disabled, true)
  await edit('')
  assert.equal(button('Save changes').disabled, false)
  assert.equal(writes, 0)
})

for (const status of [401, 403]) {
  test(`${status} clears private profile and draft after one refused PATCH`, async () => {
    allowReads()
    api.patch = (async () => {
      throw Object.assign(new Error('denied'), { response: { status } })
    }) as typeof api.patch
    const state = await mount()
    await openEditor()
    await waitFor(() => input().value === profile.display_name)
    await edit('private draft')
    await act(async () => {
      button('Save changes').click()
      await pause()
    })
    assert.equal(document.querySelector('[role="dialog"]'), null)
    assert.doesNotMatch(
      document.body.textContent ?? '',
      /private draft|fresh server name/
    )
    assert.equal(state.denied(), 1)
    assert.equal(state.updated(), 0)
  })
}

test('a same-user SID change drops late private profile data and cannot open or submit the old scope', async () => {
  let release: (() => void) | undefined
  const pending = new Promise<void>((resolve) => {
    release = resolve
  })
  api.get = (async (url: string) => {
    if (url === capabilityPath) return { data: capabilities }
    await pending
    return { data: profile }
  }) as typeof api.get
  const state = await mount()
  await openEditor()
  await act(async () => {
    login('replacement-session')
    release?.()
    await pause()
  })
  assert.equal(document.querySelector('[role="dialog"]'), null)
  assert.doesNotMatch(document.body.textContent ?? '', /fresh server name/)
  assert.equal(state.updated(), 0)
})

test('the live native page offers only the narrow ordinary-user editor and retains all 21 disabled generic actions', async () => {
  login()
  const wallet = nativeUserFixture().wallet
  assert.ok(wallet.status === 'available')
  const list = nativeListFixture([
    nativeUserFixture(),
    nativeUserFixture({
      id: '9007199254740994',
      platform_role: 'admin',
      role: 10,
      wallet: {
        ...wallet,
        provenance: { ...wallet.provenance, user_id: '9007199254740994' },
      },
    }),
    nativeUserFixture({
      id: '9007199254740995',
      platform_role: 'superadmin',
      role: 100,
      wallet: {
        ...wallet,
        provenance: { ...wallet.provenance, user_id: '9007199254740995' },
      },
    }),
  ])
  assert.doesNotThrow(() => nativeUsersResponseSchema.parse(list))
  const reads: string[] = []
  api.get = (async (url: string) => {
    reads.push(url)
    if (url === '/api/status') return { data: nativeStatusFixture }
    assert.equal(url, '/core/v1/admin/users')
    return {
      data: list,
    }
  }) as typeof api.get
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  cleanups.push(() => {
    root.unmount()
    client.clear()
    container.remove()
  })
  await act(async () => {
    root.render(
      <QueryClientProvider client={client}>
        <I18nextProvider i18n={i18n}>
          <NativeAdminUsersEntry>
            {(scope, recheck) => (
              <NativeUsersPage scope={scope} recheck={recheck} search='' />
            )}
          </NativeAdminUsersEntry>
        </I18nextProvider>
      </QueryClientProvider>
    )
    await pause()
  })
  await waitFor(
    () => container.querySelectorAll('[data-native-user-id]').length === 3
  )
  assert.equal(
    [...container.querySelectorAll('button')].filter(
      (value) => value.textContent === 'Edit · Display name'
    ).length,
    1
  )
  assert.equal(
    container.querySelectorAll('details button[disabled]').length,
    21
  )
  assert.deepEqual(reads, ['/api/status', '/core/v1/admin/users'])
})
