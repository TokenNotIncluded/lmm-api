/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, afterEach, test } from 'node:test'

import { Window } from 'happy-dom'

import {
  usageLogSchema,
  type UsageLog,
} from '@/features/usage-logs/data/schema'
import { useAuthStore } from '@/stores/auth-store'

const dom = new Window({ url: 'https://console.example.test/' })
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'SVGElement',
  'Node',
  'Element',
  'Event',
  'MutationObserver',
  'requestAnimationFrame',
  'cancelAnimationFrame',
] as const) {
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: dom[key],
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
const { LatestRequestCard } = await import('./latest-request-card')
const i18n = createInstance()
await i18n.use(initReactI18next).init({
  lng: 'en',
  resources: { en: { translation: {} } },
})
const originalAuth = useAuthStore.getState().auth
afterEach(() => useAuthStore.setState({ auth: originalAuth }))
after(() => dom.close())

async function renderCard(log: UsageLog) {
  useAuthStore.setState({
    auth: {
      ...originalAuth,
      user: {
        id: 1,
        username: 'synthetic',
        role: 1,
        developer_access_granted: true,
      },
      session: null,
    },
  })
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  client.setQueryData(['latest-user-request', 1, undefined], log)
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  await act(async () => {
    root.render(
      <QueryClientProvider client={client}>
        <I18nextProvider i18n={i18n}>
          <LatestRequestCard />
        </I18nextProvider>
      </QueryClientProvider>
    )
  })
  return {
    container,
    async cleanup() {
      await act(async () => root.unmount())
      client.clear()
      container.remove()
    },
  }
}

for (const state of ['reserved', 'streaming', 'reconcile', 'released']) {
  test(`latest ${state} receipt has no failed-response recovery action`, async () => {
    const view = await renderCard(
      usageLogSchema.parse({
        id: 1,
        user_id: 1,
        created_at: 1,
        type: 5,
        content: 'No final usage receipt',
        request_id: `synthetic-${state}`,
        quota: null,
        billing_basis: 'incomplete',
        state,
        other: JSON.stringify({
          billing_basis: 'incomplete',
          request_state: state,
          usage_status: 'incomplete',
          reserved_credits: state === 'released' ? 0 : 2000,
        }),
      })
    )
    try {
      const status = [...view.container.querySelectorAll('dt')].find(
        (element) => element.textContent === 'Status'
      )?.nextElementSibling?.textContent
      assert.equal(status, state === 'reconcile' ? 'Unknown' : 'Incomplete')
      assert.doesNotMatch(
        view.container.textContent ?? '',
        /Failed|Ask AI assistant|Successful API response observed/
      )
      assert.ok(view.container.querySelector('a[href*="request_id="]'))
    } finally {
      await view.cleanup()
    }
  })
}

test('legacy failure still offers its diagnostic action', async () => {
  const view = await renderCard(
    usageLogSchema.parse({
      id: 2,
      user_id: 1,
      created_at: 1,
      type: 5,
      content: 'Legacy error',
    })
  )
  try {
    assert.match(view.container.textContent ?? '', /Failed/)
    assert.match(view.container.textContent ?? '', /Ask AI assistant/)
  } finally {
    await view.cleanup()
  }
})
