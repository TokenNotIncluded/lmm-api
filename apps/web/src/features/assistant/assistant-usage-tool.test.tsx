/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program. If not, see <https://www.gnu.org/licenses/>.

For commercial licensing, please contact support@quantumnous.com
*/
import assert from 'node:assert/strict'
import { after, afterEach, beforeEach, describe, test } from 'node:test'

import { Window } from 'happy-dom'

const domWindow = new Window({ url: 'https://console.example.test/' })
for (const key of [
  'window',
  'document',
  'navigator',
  'history',
  'location',
  'HTMLElement',
  'HTMLButtonElement',
  'HTMLSelectElement',
  'SVGElement',
  'Node',
  'Element',
  'Event',
  'CustomEvent',
  'MutationObserver',
  'ResizeObserver',
  'requestAnimationFrame',
  'cancelAnimationFrame',
  'getComputedStyle',
  'scrollTo',
] as const) {
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: domWindow[key],
  })
}

const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { QueryClient, QueryClientProvider } =
  await import('@tanstack/react-query')
const {
  RouterProvider,
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
} = await import('@tanstack/react-router')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { api } = await import('@/lib/api')
const { useAuthStore } = await import('@/stores/auth-store')
const { AssistantUsageTool } = await import('./assistant-usage-tool')
const { resetAssistantCurrencyTest } =
  await import('./assistant-currency-test-support')

const originalGet = api.get
const reactTestGlobals = globalThis as typeof globalThis & {
  IS_REACT_ACT_ENVIRONMENT?: boolean
}
reactTestGlobals.IS_REACT_ACT_ENVIRONMENT = true

const i18n = createInstance()
await i18n.use(initReactI18next).init({
  lng: 'en',
  resources: { en: { translation: {} } },
})

async function flushQueries() {
  await new Promise((resolve) => setTimeout(resolve, 20))
}

async function renderTool(developerAccessGranted: boolean) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const rootRoute = createRootRoute({
    component: () => (
      <QueryClientProvider client={queryClient}>
        <I18nextProvider i18n={i18n}>
          <AssistantUsageTool developerAccessGranted={developerAccessGranted} />
        </I18nextProvider>
      </QueryClientProvider>
    ),
  })
  const indexRoute = createRoute({
    getParentRoute: () => rootRoute,
    path: '/',
    component: () => null,
  })
  const router = createRouter({
    routeTree: rootRoute.addChildren([indexRoute]),
    history: createMemoryHistory({ initialEntries: ['/'] }),
  })
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)

  await act(async () => {
    root.render(<RouterProvider router={router} />)
    await flushQueries()
  })
  await act(flushQueries)
  return { container, queryClient, root }
}

async function unmount(rendered: Awaited<ReturnType<typeof renderTool>>) {
  await act(async () => rendered.root.unmount())
  rendered.queryClient.clear()
  rendered.container.remove()
}

beforeEach(() => {
  resetAssistantCurrencyTest()
  useAuthStore.getState().auth.setUser({ id: 7, username: 'alice', role: 1 })
})
afterEach(async () => {
  api.get = originalGet
  document.body.replaceChildren()
  await i18n.changeLanguage('en')
})

after(() => domWindow.close())

describe('AssistantUsageTool', () => {
  test('summarizes live account usage and changes the requested range', async () => {
    const requestedDays: number[] = []
    api.get = (async (url: string, config?: Record<string, unknown>) => {
      assert.equal(url, '/api/data/self')
      const params = config?.params as {
        start_timestamp: number
        end_timestamp: number
        default_time: string
      }
      assert.equal(params.default_time, 'day')
      requestedDays.push(
        Math.round((params.end_timestamp - params.start_timestamp) / 86_400)
      )
      return {
        data: {
          success: true,
          data: [
            {
              created_at: 1,
              model_name: 'deepseek-v4-flash',
              count: 3,
              token_used: 1_500,
              quota: 1_000_000,
            },
            {
              created_at: 2,
              model_name: 'claude-sonnet-4',
              count: 1,
              token_used: 500,
              quota: 500_000,
            },
          ],
        },
      }
    }) as typeof api.get

    const rendered = await renderTool(true)
    try {
      const text = rendered.container.textContent ?? ''
      assert.match(text, /Usage at a glance/)
      assert.match(text, /deepseek-v4-flash/)
      assert.match(text, /claude-sonnet-4/)
      assert.match(text, /3 USD/)
      assert.deepEqual(requestedDays, [30])

      const select = rendered.container.querySelector<HTMLSelectElement>(
        'select[aria-label="Historical Usage"]'
      )
      assert.ok(select)
      await act(async () => {
        select.value = '7'
        select.dispatchEvent(new Event('change', { bubbles: true }))
        await flushQueries()
      })
      await act(flushQueries)
      assert.deepEqual(requestedDays, [30, 7])
    } finally {
      await unmount(rendered)
    }
  })

  test('uses language defaults and preserves manual preference across language changes', async () => {
    let calls = 0
    api.get = (async () => {
      calls += 1
      return {
        data: {
          success: true,
          data: [
            {
              created_at: 1,
              model_name: 'model-a',
              quota: 3_500_000,
              count: 1,
            },
          ],
        },
      }
    }) as typeof api.get
    await i18n.changeLanguage('zhTW')
    const rendered = await renderTool(true)
    try {
      assert.match(rendered.container.textContent ?? '', /49 CNY/)
      await act(async () => i18n.changeLanguage('en'))
      assert.match(rendered.container.textContent ?? '', /7 USD/)
      await act(async () => {
        useAuthStore.getState().auth.setUser({
          id: 7,
          username: 'alice',
          role: 1,
          setting: { wallet_display_currency: 'CREDIT' },
        })
      })
      assert.match(rendered.container.textContent ?? '', /3,500,000 Credits/)
      await act(async () => i18n.changeLanguage('zhCN'))
      assert.match(rendered.container.textContent ?? '', /3,500,000 Credits/)
      assert.equal(calls, 1)
      assert.doesNotMatch(rendered.container.textContent ?? '', /\(Platform\)/)
    } finally {
      await unmount(rendered)
    }
  })

  test('keeps L0 restricted without requesting account usage', async () => {
    let calls = 0
    api.get = (async () => {
      calls += 1
      throw new Error('usage endpoint must stay disabled')
    }) as typeof api.get

    const rendered = await renderTool(false)
    try {
      assert.equal(calls, 0)
      assert.match(rendered.container.textContent ?? '', /Historical Usage/)
      assert.match(rendered.container.textContent ?? '', /access is activated/)
    } finally {
      await unmount(rendered)
    }
  })
})

test('image usage shows its billing quantity and sponsor without inventing token zero', async () => {
  api.get = (async () => ({
    data: {
      success: true,
      data: [
        {
          user_id: 7,
          model_name: 'image-native',
          created_at: 1,
          count: 1,
          token_used: null,
          output_image_count: 2,
          quota: 2000,
          quota_usd: '0.004000',
          quota_money_available: true,
          quota_money_basis: 'captured_credits_per_usd',
          token_count_scope: 'settled_text_requests',
          usage_scope: 'actor_personal_usage',
          settlement_scope: 'settled_requests',
          platform_sponsored: true,
          payer_kind: 'personal',
          subscription_funded: false,
          gross_settled_credits: 2000,
          refunded_credits: 0,
          net_consumption_credits: 2000,
          billing_basis: 'net_consumption',
          billing_currency: 'USD',
          wallet_debit_supported: false,
        },
      ],
    },
  })) as typeof api.get
  const rendered = await renderTool(true)
  try {
    const text = rendered.container.textContent ?? ''
    assert.match(text, /Total tokensNot recorded/)
    assert.match(text, /Images \(billing quantity\): 2/)
    assert.match(text, /Tokens cover settled text requests only\./)
    assert.match(text, /Platform-sponsored usage/)
    assert.doesNotMatch(text, /wallet|debit/i)
  } finally {
    await unmount(rendered)
  }
})

test('unknown amount stays unknown while a measured zero text token count remains zero', async () => {
  api.get = (async () => ({
    data: {
      success: true,
      data: [
        {
          created_at: 1,
          count: 1,
          token_used: 0,
          quota: null,
          model_name: 'text-zero',
        },
      ],
    },
  })) as typeof api.get
  const rendered = await renderTool(true)
  try {
    const text = rendered.container.textContent ?? ''
    assert.match(text, /Total tokens0/)
    assert.match(text, /Total consumedUnknown/)
    assert.doesNotMatch(text, /No recent usage/)
  } finally {
    await unmount(rendered)
  }
})

test('a missing success payload fails instead of pretending the account has no usage', async () => {
  api.get = (async () => ({
    data: { success: true, data: null },
  })) as typeof api.get
  const rendered = await renderTool(true)
  try {
    assert.match(rendered.container.textContent ?? '', /Failed to fetch usage/)
    assert.doesNotMatch(rendered.container.textContent ?? '', /No recent usage/)
  } finally {
    await unmount(rendered)
  }
})

test('switching accounts clears the previous assistant usage while the next response is pending', async () => {
  let calls = 0
  let releaseSecond: (() => void) | undefined
  const response = (model: string) => ({
    data: {
      success: true,
      data: [
        {
          created_at: 1,
          count: 1,
          token_used: 10,
          quota: 5,
          model_name: model,
        },
      ],
    },
  })
  api.get = (async () => {
    calls += 1
    if (calls === 1) return response('account-a-model')
    return new Promise<ReturnType<typeof response>>((resolve) => {
      releaseSecond = () => resolve(response('account-b-model'))
    })
  }) as typeof api.get
  const rendered = await renderTool(true)
  try {
    assert.match(rendered.container.textContent ?? '', /account-a-model/)
    await act(async () => {
      useAuthStore.getState().auth.setUser({ id: 8, username: 'bob', role: 1 })
      await flushQueries()
    })
    assert.doesNotMatch(rendered.container.textContent ?? '', /account-a-model/)
    assert.ok(releaseSecond)
    await act(async () => {
      releaseSecond?.()
      await flushQueries()
    })
    await act(flushQueries)
    assert.match(rendered.container.textContent ?? '', /account-b-model/)
    assert.equal(calls, 2)
  } finally {
    await unmount(rendered)
  }
})
