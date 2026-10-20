/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { Window } from 'happy-dom'

const dom = new Window({ url: 'https://web.example' })
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
  'getComputedStyle',
  'requestAnimationFrame',
  'cancelAnimationFrame',
  'sessionStorage',
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
const { api } = await import('@/lib/api')
const { savePendingTransfer, readPendingTransfer } = await import('./flow')
const { WalletTransfers } = await import('./wallet-transfers')
const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: {} } } })
const flush = () => new Promise((resolve) => setTimeout(resolve, 30))

test('reopened confirmation resumes original key with GET, hides unavailable data and opens only Core page', async () => {
  const oldGet = api.get,
    oldPost = api.post
  const root = createRoot(
    document.body.appendChild(document.createElement('div'))
  )
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const originalKey = 'immutable-wallet-key'
  let writes = 0,
    confirmed = false,
    opened = ''
  const row = {
    id: 17,
    quota: 125,
    status: 'awaiting_confirmation',
    created_at: 1,
    request_key: originalKey,
    claim_link_available: false,
    recipient_details_available: false,
  }
  savePendingTransfer(11, {
    action: 'create',
    requestKey: originalKey,
    quota: 125,
    step: { kind: 'unknown' },
  })
  dom.open = ((url?: string) => {
    opened = url ?? ''
    return null
  }) as typeof dom.open
  api.post = (async () => {
    writes++
    throw new Error('unexpected financial replay')
  }) as typeof api.post
  api.get = (async (url: string, options: Record<string, unknown>) => {
    if (url === '/api/wallet-transfer/query') {
      assert.deepEqual(options.params, { request_key: originalKey })
      return {
        data: confirmed
          ? { success: true, data: { ...row, status: 'pending' } }
          : {
              success: false,
              code: 'WALLET_CONFIRMATION_REQUIRED',
              data: {
                request_key: originalKey,
                transfer: row,
                confirmation_url:
                  'https://web.example/core/v1/wallet/confirm/create/17',
              },
            },
      }
    }
    return {
      data: {
        success: true,
        data: [
          {
            ...row,
            status: 'claimed',
            recipient_id: 9,
            recipient_name: 'hidden-recipient',
            recipient_email: 'hidden@example.com',
          },
        ],
      },
    }
  }) as typeof api.get
  const click = async (label: string) => {
    const button = Array.from(document.querySelectorAll('button')).find(
      (node) => node.textContent?.trim() === label
    )
    assert.ok(button, label)
    await act(async () => {
      button.click()
      await flush()
    })
  }
  try {
    await act(async () => {
      root.render(
        <I18nextProvider i18n={i18n}>
          <QueryClientProvider client={client}>
            <WalletTransfers
              userID={11}
              balance={1000}
              onBalanceChange={async () => {}}
            />
          </QueryClientProvider>
        </I18nextProvider>
      )
      await flush()
    })
    await click('Transfer')
    await act(flush)
    assert.equal(
      (document.getElementById('transfer-amount') as HTMLInputElement).disabled,
      true
    )
    assert.ok(document.body.textContent?.includes('Awaiting confirmation'))
    assert.equal(document.body.textContent?.includes('hidden-recipient'), false)
    assert.equal(
      document.body.textContent?.includes('hidden@example.com'),
      false
    )
    assert.equal(document.body.textContent?.includes('Link and QR code'), false)
    await click('Continue')
    assert.equal(opened, 'https://web.example/core/v1/wallet/confirm/create/17')
    assert.equal(readPendingTransfer(11)?.requestKey, originalKey)
    confirmed = true
    const refreshes = Array.from(document.querySelectorAll('button')).find(
      (node) => node.textContent?.trim() === 'Refresh'
    )
    assert.ok(refreshes)
    await act(async () => {
      refreshes.click()
      await flush()
    })
    assert.equal(readPendingTransfer(11), null)
    assert.equal(document.body.textContent?.includes('Copy link'), false)
    assert.equal(writes, 0)
  } finally {
    await act(async () => root.unmount())
    client.clear()
    api.get = oldGet
    api.post = oldPost
    savePendingTransfer(11, null)
    dom.happyDOM.abort()
  }
})
