/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, afterEach, test } from 'node:test'

import { Window } from 'happy-dom'

const dom = new Window({
  url: 'https://console.example.test/usage-logs/common',
})
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'SVGElement',
  'Node',
  'Element',
  'Event',
  'CustomEvent',
  'MutationObserver',
  'requestAnimationFrame',
  'cancelAnimationFrame',
  'getComputedStyle',
  'scrollTo',
] as const) {
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: dom[key],
  })
}
Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true })
after(() => dom.close())

const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { QueryClient, QueryClientProvider } =
  await import('@tanstack/react-query')
const {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  RouterProvider,
} = await import('@tanstack/react-router')
const { createInstance } = await import('i18next')
const { initReactI18next, I18nextProvider } = await import('react-i18next')
const { CommonLogsStats } = await import('../common-logs-stats')
const { UsageLogsProvider, useLogsViewScope } =
  await import('../usage-logs-provider')
const { api } = await import('@/lib/api')
const { useAuthStore } = await import('@/stores/auth-store')
const { createQueryRetry } = await import('@/lib/query-retry')
const originalAdapter = api.defaults.adapter
const cleanups: Array<() => void> = []

const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: {}, fallbackLng: 'en' })

function Fixture() {
  const { setViewScope } = useLogsViewScope()
  return (
    <>
      <button type='button' onClick={() => setViewScope('self')}>
        Only mine
      </button>
      <CommonLogsStats />
    </>
  )
}

async function pause() {
  await new Promise((resolve) => setTimeout(resolve, 20))
}
async function waitFor(condition: () => boolean) {
  for (let index = 0; index < 60 && !condition(); index += 1) await act(pause)
  assert.ok(condition())
}
function button(container: HTMLElement, label: string) {
  const found = [
    ...container.querySelectorAll<HTMLButtonElement>('button'),
  ].find((value) => value.textContent === label)
  assert.ok(found, `Missing button ${label}`)
  return found
}
function badgesUnavailable(container: HTMLElement) {
  for (const label of ['Usage', 'RPM', 'TPM']) {
    assert.match(container.textContent ?? '', new RegExp(`${label}\\s*—`))
  }
}
function isSummary(url: string) {
  const query = new URL(url, 'https://console.example.test').searchParams
  return (
    Number(query.get('end_timestamp')) -
      Number(query.get('start_timestamp')) ===
    86400
  )
}

async function mount() {
  const rootRoute = createRootRoute()
  const authRoute = createRoute({
    getParentRoute: () => rootRoute,
    id: '_authenticated',
  })
  const logsRoute = createRoute({
    getParentRoute: () => authRoute,
    path: '/usage-logs/$section',
    component: Fixture,
    validateSearch: (search: Record<string, unknown>) => search,
  })
  const router = createRouter({
    routeTree: rootRoute.addChildren([authRoute.addChildren([logsRoute])]),
    history: createMemoryHistory({
      initialEntries: [
        '/usage-logs/common?startTime=1704067200000&endTime=1704153600000',
      ],
    }),
  })
  const client = new QueryClient({
    defaultOptions: {
      queries: { retry: createQueryRetry(true), retryDelay: 0 },
    },
  })
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  cleanups.push(() => {
    root.unmount()
    client.clear()
    container.remove()
  })
  await act(async () => {
    await router.load()
    root.render(
      <I18nextProvider i18n={i18n}>
        <QueryClientProvider client={client}>
          <UsageLogsProvider>
            <RouterProvider router={router} />
          </UsageLogsProvider>
        </QueryClientProvider>
      </I18nextProvider>
    )
    await pause()
  })
  return { container, router, client }
}

afterEach(async () => {
  await act(async () => {
    for (const cleanup of cleanups.splice(0)) cleanup()
  })
  api.defaults.adapter = originalAdapter
  useAuthStore.getState().auth.reset('idle')
})

for (const failure of [
  'business',
  'missing',
  'invalid',
  401,
  403,
  503,
] as const) {
  test(`${failure} statistics do not become zero and recover only through an explicit read`, async () => {
    useAuthStore
      .getState()
      .auth.setUser({ id: 7, username: 'synthetic', role: 1 })
    let available = false
    let requests = 0
    api.defaults.adapter = async (config) => {
      requests += 1
      let data: unknown = { success: true, data: { quota: 0, rpm: 0, tpm: 0 } }
      if (!available) {
        if (typeof failure === 'number') {
          throw Object.assign(new Error(`HTTP ${failure}`), {
            config,
            response: { status: failure },
          })
        }
        data =
          failure === 'business'
            ? { success: false }
            : failure === 'missing'
              ? { success: true }
              : { success: true, data: { quota: '0', rpm: 0, tpm: 0 } }
      }
      return { config, data, headers: {}, status: 200, statusText: 'OK' }
    }
    const { container } = await mount()
    await waitFor(
      () => container.textContent?.includes('Failed to load') === true
    )
    badgesUnavailable(container)
    assert.equal(
      requests,
      13,
      'local retry:false overrides the production retry default'
    )
    assert.doesNotMatch(container.textContent ?? '', /No usage trend data/)
    available = true
    await act(async () => {
      button(container, 'Retry').click()
      await pause()
    })
    await waitFor(
      () =>
        container.textContent?.includes('No usage trend data in this range') ===
        true
    )
    assert.match(container.textContent ?? '', /RPM\s*0/)
    assert.match(container.textContent ?? '', /TPM\s*0/)
    assert.match(container.textContent ?? '', /0 Raw credits/)
    assert.doesNotMatch(container.textContent ?? '', /Failed to load|USD/)
    assert.equal(requests, 26)
  })
}

test('one failed trend bucket makes the whole trend unavailable instead of drawing eleven complete bars', async () => {
  useAuthStore
    .getState()
    .auth.setUser({ id: 7, username: 'synthetic', role: 1 })
  let failBucket = true
  let bucketReads = 0
  api.defaults.adapter = async (config) => {
    const summary = isSummary(config.url ?? '')
    if (!summary) bucketReads += 1
    const data =
      !summary && failBucket && bucketReads === 4
        ? { success: false }
        : { success: true, data: { quota: 100, rpm: 3, tpm: 9 } }
    return { config, data, headers: {}, status: 200, statusText: 'OK' }
  }
  const { container } = await mount()
  await waitFor(
    () => container.textContent?.includes('Unable to load usage trend') === true
  )
  assert.equal(
    container.querySelector(
      '[aria-label="Usage trend for the selected time range"]'
    ),
    null
  )
  assert.match(container.textContent ?? '', /100 Raw credits/)
  assert.equal(bucketReads, 12)
  failBucket = false
  await act(async () => {
    button(container, 'Retry').click()
    await pause()
  })
  await waitFor(
    () =>
      container.querySelectorAll(
        '[aria-label="Usage trend for the selected time range"] button'
      ).length === 12
  )
  assert.equal(bucketReads, 24)
  assert.doesNotMatch(container.textContent ?? '', /USD/)
})

test('scope, filter and account changes cannot show the preceding successful financial query', async () => {
  useAuthStore
    .getState()
    .auth.setUser({ id: 7, username: 'synthetic', role: 100 })
  let available = true
  let amount = 321
  const endpoints: string[] = []
  api.defaults.adapter = async (config) => {
    endpoints.push(config.url ?? '')
    return {
      config,
      headers: {},
      status: 200,
      statusText: 'OK',
      data: available
        ? { success: true, data: { quota: amount, rpm: 3, tpm: 9 } }
        : { success: false },
    }
  }
  const { container, router } = await mount()
  await waitFor(
    () => container.textContent?.includes('321 Raw credits') === true
  )
  available = false
  await act(async () => {
    button(container, 'Only mine').click()
    await pause()
  })
  await waitFor(
    () => container.textContent?.includes('Failed to load') === true
  )
  badgesUnavailable(container)
  assert.doesNotMatch(container.textContent ?? '', /321/)
  assert.ok(endpoints.some((url) => url.startsWith('/api/log/self/stat?')))

  available = true
  amount = 876
  await act(async () => {
    button(container, 'Retry').click()
    await pause()
  })
  await waitFor(
    () => container.textContent?.includes('876 Raw credits') === true
  )
  available = false
  await act(async () => {
    router.history.push(
      '/usage-logs/common?startTime=1704153600000&endTime=1704240000000'
    )
    await pause()
  })
  await waitFor(
    () => container.textContent?.includes('Failed to load') === true
  )
  badgesUnavailable(container)
  assert.doesNotMatch(container.textContent ?? '', /876/)

  await act(async () => {
    useAuthStore.getState().auth.setUser({ id: 8, username: 'other', role: 1 })
    await pause()
  })
  await waitFor(
    () => container.textContent?.includes('Failed to load') === true
  )
  badgesUnavailable(container)
  assert.doesNotMatch(container.textContent ?? '', /321|876/)
})

test('late responses from the previous account never overwrite the new account statistics', async () => {
  useAuthStore.getState().auth.setUser({ id: 7, username: 'first', role: 1 })
  let release: (() => void) | undefined
  const pending = new Promise<void>((resolve) => {
    release = resolve
  })
  api.defaults.adapter = async (config) => {
    const prior = config.authScope?.userId === 7
    if (prior) await pending
    return {
      config,
      headers: {},
      status: 200,
      statusText: 'OK',
      data: { success: true, data: { quota: prior ? 999 : 0, rpm: 0, tpm: 0 } },
    }
  }
  const { container } = await mount()
  await act(async () => {
    useAuthStore.getState().auth.setUser({ id: 8, username: 'second', role: 1 })
    await pause()
  })
  await waitFor(() => container.textContent?.includes('0 Raw credits') === true)
  await act(async () => {
    release?.()
    await pause()
  })
  assert.doesNotMatch(container.textContent ?? '', /999/)
  assert.match(container.textContent ?? '', /0 Raw credits/)
})

// Independent acceptance found these contradictory provider projections after v2.
for (const money of [
  { quota: 817, quota_usd: '0', quota_money_basis: 'empty_result' },
  {
    quota: 0,
    quota_usd: '7.250000',
    quota_money_basis: 'captured_credits_per_usd',
  },
]) {
  test(`contradictory ${money.quota_money_basis} money retains raw counters instead of a fabricated complete amount`, async () => {
    useAuthStore
      .getState()
      .auth.setUser({ id: 7, username: 'synthetic', role: 1 })
    api.defaults.adapter = async (config) => ({
      config,
      headers: {},
      status: 200,
      statusText: 'OK',
      data: {
        success: true,
        data: { ...money, quota_money_available: true, rpm: 29, tpm: 743 },
      },
    })
    const { container } = await mount()
    await waitFor(
      () =>
        container.textContent?.includes(`${money.quota} Raw credits`) === true
    )
    assert.match(container.textContent ?? '', /RPM\s*29/)
    assert.match(container.textContent ?? '', /TPM\s*743/)
    assert.doesNotMatch(container.textContent ?? '', /USD/)
    assert.match(
      container.textContent ?? '',
      /Historical monetary value unavailable/
    )
  })
}
