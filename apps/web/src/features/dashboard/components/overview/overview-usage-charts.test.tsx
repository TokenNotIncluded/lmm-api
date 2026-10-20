/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, afterEach, test } from 'node:test'

import { Window } from 'happy-dom'

const dom = new Window({ url: 'http://localhost/' })
for (const [key, value] of Object.entries({
  window: dom,
  document: dom.document,
  navigator: dom.navigator,
  HTMLElement: dom.HTMLElement,
  Element: dom.Element,
  Node: dom.Node,
  MutationObserver: dom.MutationObserver,
  getComputedStyle: dom.getComputedStyle,
  localStorage: dom.localStorage,
  sessionStorage: dom.sessionStorage,
  requestAnimationFrame: dom.requestAnimationFrame,
  cancelAnimationFrame: dom.cancelAnimationFrame,
  IS_REACT_ACT_ENVIRONMENT: true,
})) {
  Object.defineProperty(globalThis, key, { configurable: true, value })
}

const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { QueryClient, QueryClientProvider } =
  await import('@tanstack/react-query')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { useAuthStore } = await import('@/stores/auth-store')
const { OverviewUsageCharts } = await import('./overview-usage-charts')
const { overviewUsageRange } = await import('./overview-personalization-data')
const originalUser = useAuthStore.getState().auth.user
const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: {} } } })
let dispose: (() => Promise<void>) | undefined

afterEach(async () => {
  await dispose?.()
  dispose = undefined
  useAuthStore.getState().auth.setUser(originalUser)
  document.body.replaceChildren()
})
after(() => dom.close())

test('the actual all-image token chart renders unavailable instead of a fabricated zero line', async () => {
  useAuthStore
    .getState()
    .auth.setUser({ id: 881, username: 'image-owner', role: 1 })
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const range = overviewUsageRange(7)
  client.setQueryData(['overview-recorded-usage', 881, 7], {
    ...range,
    rows: [
      {
        created_at: range.end_timestamp,
        count: 1,
        quota: 2000,
        model_name: 'image-model',
        token_used: null,
        output_image_count: 2,
        token_count_scope: 'settled_text_requests',
      },
    ],
  })
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  act(() =>
    root.render(
      <QueryClientProvider client={client}>
        <I18nextProvider i18n={i18n}>
          <OverviewUsageCharts />
        </I18nextProvider>
      </QueryClientProvider>
    )
  )
  dispose = async () => {
    await act(async () => root.unmount())
    client.clear()
  }
  const tokensButton = [...container.querySelectorAll('button')].find(
    (button) => button.textContent === 'Tokens'
  )
  assert.ok(tokensButton)
  await act(async () => tokensButton.click())
  assert.match(
    container.querySelector('[role="status"]')?.textContent ?? '',
    /Unavailable/
  )
  assert.match(
    container.textContent ?? '',
    /Tokens cover settled text requests only\./
  )
  assert.match(container.textContent ?? '', /Images \(billing quantity\): 2/)
  assert.equal(container.querySelector('.overview-chart-grid'), null)
  assert.equal(container.querySelector('[role="alert"]'), null)
})
