/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, afterEach, test } from 'node:test'

import { Window } from 'happy-dom'

import type { QuotaDataItem } from '../../types'

const dom = new Window({ url: 'http://localhost/' })
for (const [key, value] of Object.entries({
  window: dom,
  document: dom.document,
  navigator: dom.navigator,
  localStorage: dom.localStorage,
  sessionStorage: dom.sessionStorage,
  HTMLElement: dom.HTMLElement,
  Element: dom.Element,
  Node: dom.Node,
  MutationObserver: dom.MutationObserver,
  getComputedStyle: dom.getComputedStyle,
  requestAnimationFrame: dom.requestAnimationFrame,
  cancelAnimationFrame: dom.cancelAnimationFrame,
  IS_REACT_ACT_ENVIRONMENT: true,
})) {
  Object.defineProperty(globalThis, key, { configurable: true, value })
}

const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { api } = await import('@/lib/api')
const { LogStatCards } = await import('./log-stat-cards')
const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: {} } } })
const originalAdapter = api.defaults.adapter
let dispose: (() => Promise<void>) | undefined

afterEach(async () => {
  await dispose?.()
  dispose = undefined
  api.defaults.adapter = originalAdapter
  dom.localStorage.clear()
  dom.sessionStorage.clear()
  document.body.replaceChildren()
})
after(() => dom.close())

const image: QuotaDataItem = {
  user_id: 1,
  model_name: 'native-image',
  created_at: 1,
  count: 1,
  quota: 2000,
  token_used: null,
  output_image_count: 2,
  token_count_scope: 'settled_text_requests',
  usage_scope: 'actor_personal_usage',
  settlement_scope: 'settled_requests',
  platform_sponsored: true,
  payer_kind: 'personal',
  subscription_funded: false,
  quota_usd: '0.004000',
  quota_money_available: true,
  quota_money_basis: 'captured_credits_per_usd',
  gross_settled_credits: 2000,
  refunded_credits: 0,
  net_consumption_credits: 2000,
  billing_basis: 'net_consumption',
  billing_currency: 'USD',
  wallet_debit_supported: false,
}
const text: QuotaDataItem = {
  ...image,
  model_name: 'native-text',
  count: 2,
  quota: 7,
  token_used: 193,
  output_image_count: 0,
  platform_sponsored: false,
  quota_usd: '0.000014',
  gross_settled_credits: 7,
  net_consumption_credits: 7,
}

async function mount(rows: QuotaDataItem[]) {
  api.defaults.adapter = async (config) => ({
    config,
    data: { success: true, data: rows },
    headers: {},
    status: 200,
    statusText: 'OK',
  })
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  await act(async () =>
    root.render(
      <I18nextProvider i18n={i18n}>
        <LogStatCards
          filters={{
            start_timestamp: new Date('2026-10-20T00:00:00Z'),
            end_timestamp: new Date('2026-10-20T00:02:00Z'),
          }}
        />
      </I18nextProvider>
    )
  )
  dispose = async () => act(async () => root.unmount())
  for (let i = 0; i < 100; i++) {
    if (container.querySelector('[aria-busy="false"]')) {
      assert.equal(container.querySelector('[role="alert"]'), null)
      return container
    }
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 10))
    })
  }
  assert.fail('Usage cards did not finish loading')
}

function card(container: HTMLElement, title: string) {
  const result = [...container.querySelectorAll('.dashboard-stat')].find(
    (item) => item.querySelector('dt')?.textContent === title
  )
  assert.ok(result, `Missing ${title} card`)
  return {
    value: result.querySelector('.dashboard-stat__value')?.textContent,
    hint: result.querySelector('.dashboard-stat__hint')?.textContent ?? '',
  }
}

test('image-only cards display unavailable tokens while preserving calls, consumption and images', async () => {
  const container = await mount([image])
  assert.equal(card(container, 'Total Tokens').value, '—')
  assert.equal(card(container, 'Average TPM').value, '—')
  assert.match(card(container, 'Total Tokens').hint, /Unavailable/)
  assert.match(
    card(container, 'Total Tokens').hint,
    /Tokens cover settled text requests only\./
  )
  assert.equal(card(container, 'Total Count').value, '1')
  assert.equal(card(container, 'Raw credits').value, '2,000 Raw credits')
  assert.equal(card(container, 'Images (billing quantity)').value, '2')
  assert.match(card(container, 'Raw credits').hint, /Platform-sponsored usage/)
  assert.doesNotMatch(
    container.textContent ?? '',
    /wallet debit|wallet deduction/i
  )
})

test('mixed image and text cards show the known text total and average', async () => {
  const container = await mount([image, text])
  assert.equal(card(container, 'Total Tokens').value, '193')
  assert.equal(card(container, 'Average TPM').value, '96.5')
  assert.equal(card(container, 'Total Count').value, '3')
  assert.equal(card(container, 'Raw credits').value, '2,007 Raw credits')
  assert.equal(card(container, 'Images (billing quantity)').value, '2')
  assert.doesNotMatch(card(container, 'Total Tokens').hint, /Unavailable/)
})

test('measured text token zero remains zero in both token cards', async () => {
  const container = await mount([{ ...text, token_used: 0 }])
  assert.equal(card(container, 'Total Tokens').value, '0')
  assert.equal(card(container, 'Average TPM').value, '0')
  assert.doesNotMatch(card(container, 'Total Tokens').hint, /Unavailable/)
})

test('unexplained token gaps remain unavailable even beside measured text', async () => {
  const container = await mount([
    { created_at: 1, count: 1, quota: 2000 },
    text,
  ])
  assert.equal(card(container, 'Total Tokens').value, '—')
  assert.equal(card(container, 'Average TPM').value, '—')
  assert.equal(card(container, 'Total Count').value, '3')
  assert.equal(card(container, 'Raw credits').value, '2,007 Raw credits')
})

test('unknown legacy consumption is unavailable without hiding its measured tokens and requests', async () => {
  const container = await mount([
    { created_at: 1, count: 1, quota: null, token_used: 12 },
  ])
  assert.equal(card(container, 'Raw credits').value, '—')
  assert.match(card(container, 'Raw credits').hint, /Unavailable/)
  assert.equal(card(container, 'Total Tokens').value, '12')
  assert.equal(card(container, 'Total Count').value, '1')
})

test('cache cards show measured totals without double-counting lifetime details', async () => {
  const container = await mount([
    image,
    {
      ...text,
      cache_read_tokens: 12,
      cache_creation_tokens: 20,
      cache_creation_tokens_12h: 15,
    },
  ])
  assert.equal(card(container, 'Cache read tokens').value, '12')
  assert.equal(card(container, 'Cache write tokens').value, '20')
  assert.equal(card(container, 'Total Tokens').value, '193')
})

test('cache cards preserve measured zero and mark missing legacy counters unavailable', async () => {
  const container = await mount([
    { ...text, cache_read_tokens: 0, cache_creation_tokens: null },
  ])
  assert.equal(card(container, 'Cache read tokens').value, '0')
  assert.equal(card(container, 'Cache write tokens').value, '—')
  assert.match(card(container, 'Cache write tokens').hint, /Unavailable/)
})
