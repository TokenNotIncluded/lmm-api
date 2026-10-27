/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, afterEach, test } from 'node:test'

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
const { ReadOnlyChannels } = await import('./read-only-channels')
const originalGet = api.get
const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: {} } } })
let dispose: (() => Promise<void>) | undefined
afterEach(async () => {
  await dispose?.()
  dispose = undefined
  api.get = originalGet
  document.body.replaceChildren()
})
after(() => dom.close())

async function mount() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  await act(async () =>
    root.render(
      <QueryClientProvider client={client}>
        <I18nextProvider i18n={i18n}>
          <ReadOnlyChannels />
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
  assert.ok(check(), 'The channel view did not reach the expected state')
}

test('read-only channels show snapshot fields and paginate without mutation controls or fake metrics', async () => {
  api.get = (async (_url: string, config?: Parameters<typeof api.get>[1]) => {
    const page = Number(config?.params?.p)
    return {
      data: {
        success: true,
        data: {
          items: [
            {
              id: page,
              name: `Channel ${page}`,
              status: page === 1 ? 1 : 2,
              group: 'vip',
              models: 'native-model',
            },
          ],
          total: 21,
        },
      },
    }
  }) as typeof api.get
  const container = await mount()
  await until(() =>
    Boolean(
      container.querySelector('table')?.textContent?.includes('Channel 1')
    )
  )
  assert.match(container.textContent ?? '', /Read only/)
  assert.match(container.textContent ?? '', /Total: 21/)
  assert.match(container.querySelector('tbody')?.textContent ?? '', /Enabled/)
  assert.deepEqual(
    [...container.querySelectorAll('button')].map(
      (button) => button.textContent
    ),
    ['Search', 'Previous', 'Next']
  )
  assert.doesNotMatch(
    container.querySelector('table')?.textContent ?? '',
    /Balance|Response Time|Actions/
  )
  const next = [...container.querySelectorAll('button')].find(
    (button) => button.textContent === 'Next'
  )
  assert.ok(next)
  await act(async () => next.click())
  await until(() =>
    Boolean(
      container.querySelector('table')?.textContent?.includes('Channel 2')
    )
  )
  assert.match(container.querySelector('tbody')?.textContent ?? '', /Disabled/)
})

test('read-only channel search submits a trimmed keyword to the read endpoint', async () => {
  const requests: { url: string; keyword?: string; page: number }[] = []
  api.get = (async (url: string, config?: Parameters<typeof api.get>[1]) => {
    requests.push({
      url,
      keyword: config?.params?.keyword,
      page: config?.params?.p,
    })
    return {
      data: {
        success: true,
        data: {
          items: [
            {
              id: 1,
              name: 'Snapshot channel',
              status: 1,
              group: 'vip',
              models: 'model',
            },
          ],
          total: 1,
        },
      },
    }
  }) as typeof api.get
  const container = await mount()
  await until(() => Boolean(container.querySelector('table')))
  const input = container.querySelector('input')
  const form = container.querySelector('form')
  assert.ok(input)
  assert.ok(form)
  const setter = Object.getOwnPropertyDescriptor(
    dom.HTMLInputElement.prototype,
    'value'
  )?.set
  assert.ok(setter)
  await act(async () => {
    setter.call(input, '  model  ')
    input.dispatchEvent(new Event('input', { bubbles: true }))
  })
  await act(async () => {
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }))
  })
  await until(() =>
    requests.some((request) => request.url === '/api/channel/search')
  )
  assert.deepEqual(requests.at(-1), {
    url: '/api/channel/search',
    keyword: 'model',
    page: 1,
  })
})

test('read-only channel failures keep the server reason instead of an empty successful list', async () => {
  api.get = (async () => {
    throw Object.assign(new Error('Request failed with status code 503'), {
      response: {
        data: { error: { message: 'Active routing snapshot is unavailable' } },
      },
    })
  }) as typeof api.get
  const container = await mount()
  await until(() =>
    Boolean(
      container.textContent?.includes('Active routing snapshot is unavailable')
    )
  )
  assert.equal(container.querySelector('table'), null)
  assert.doesNotMatch(container.textContent ?? '', /Total: 0|No data available/)
  assert.ok(
    [...container.querySelectorAll('button')].find(
      (button) => button.textContent === 'Retry'
    )
  )
})
