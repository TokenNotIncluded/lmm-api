/*
Copyright (C) 2026 LIghtJUNction
*/
import assert from 'node:assert/strict'
import { after, test } from 'node:test'

import { Window } from 'happy-dom'

import type { UsageBucket } from '@/features/dashboard/lib/usage-bucket'

const dom = new Window({ url: 'https://console.example.test/profile/share' })
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
  'scrollTo',
] as const) {
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: dom[key],
  })
}
Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true })
const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { QueryClient, QueryClientProvider } =
  await import('@tanstack/react-query')
const { RouterProvider, createMemoryHistory, createRootRoute, createRouter } =
  await import('@tanstack/react-router')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { ThemeProvider } = await import('@/context/theme-provider')
const { api } = await import('@/lib/api')
const { useAuthStore } = await import('@/stores/auth-store')
const { DEFAULT_CURRENCY_CONFIG, useSystemConfigStore } =
  await import('@/stores/system-config-store')
const { ModelUsageReport } = await import('./model-usage-report')
const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: {} } } })
after(() => dom.close())

test('model usage and copied report update units and FX without refetching or changing Credits', async () => {
  const previousAuth = useAuthStore.getState().auth
  const previousCurrency = useSystemConfigStore.getState().config.currency
  const previousGet = api.get
  let calls = 0
  let markdown = ''
  const updateCopy = (snapshot: { markdown: string } | null) => {
    markdown = snapshot?.markdown ?? ''
  }
  const user = {
    id: 7,
    username: 'alice',
    role: 1,
    setting: { wallet_display_currency: 'USD' },
  }
  useAuthStore.getState().auth.setUser(user)
  useSystemConfigStore.getState().setConfig({
    currency: {
      ...DEFAULT_CURRENCY_CONFIG,
      currencyUnit: 'credit',
      creditsPerUsd: 500_000,
      creditsPerUsdExact: '500000',
      cnyPerUsd: 7,
      cnyPerUsdExact: '7',
    },
  })
  api.get = (async () => {
    calls += 1
    return {
      data: {
        success: true,
        data: [
          {
            created_at: 1_720_000_000,
            model_name: 'model-a',
            quota: 3_500_001,
            token_used: 100,
            count: 1,
          },
        ],
      },
    }
  }) as typeof api.get
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const host = document.createElement('div')
  document.body.append(host)
  const root = createRoot(host)
  const route = createRootRoute({
    component: () => (
      <QueryClientProvider client={client}>
        <I18nextProvider i18n={i18n}>
          <ThemeProvider defaultTheme='light'>
            <ModelUsageReport rangeKey='7d' onCopySnapshotChange={updateCopy} />
          </ThemeProvider>
        </I18nextProvider>
      </QueryClientProvider>
    ),
  })
  const router = createRouter({
    routeTree: route,
    history: createMemoryHistory({ initialEntries: ['/'] }),
  })
  try {
    await act(async () => {
      root.render(<RouterProvider router={router} />)
    })
    for (let attempt = 0; !markdown && attempt < 30; attempt += 1) {
      await act(async () => {
        await new Promise((resolve) => setTimeout(resolve, 5))
      })
    }
    assert.ok(markdown, 'the copied report did not become available')
    assert.match(markdown, /7 USD/)
    await act(async () =>
      useAuthStore
        .getState()
        .auth.setUser({ ...user, setting: { wallet_display_currency: 'CNY' } })
    )
    assert.match(markdown, /49 CNY/)
    assert.match(host.textContent ?? '', /49 CNY/)
    await act(async () =>
      useSystemConfigStore.getState().setConfig({
        currency: {
          ...useSystemConfigStore.getState().config.currency,
          cnyPerUsd: 8,
          cnyPerUsdExact: '8',
        },
      })
    )
    assert.match(markdown, /56 CNY/)
    await act(async () =>
      useAuthStore.getState().auth.setUser({
        ...user,
        setting: { wallet_display_currency: 'CREDIT' },
      })
    )
    assert.match(markdown, /3,500,001 Credits/)
    await act(async () => i18n.changeLanguage('zhCN'))
    assert.match(markdown, /3,500,001 Credits/)
    assert.equal(calls, 1)
    assert.doesNotMatch(markdown, /Platform|\$/)
  } finally {
    await act(async () => root.unmount())
    client.clear()
    host.remove()
    api.get = previousGet
    useAuthStore.setState({ auth: previousAuth })
    useSystemConfigStore.getState().setConfig({ currency: previousCurrency })
  }
})

function nativeBucket({
  modelName,
  tokens,
  images,
  sponsored,
  credits,
}: {
  modelName: string
  tokens: number | null
  images: number
  sponsored: boolean
  credits: number
}): UsageBucket {
  return {
    user_id: 42,
    model_name: modelName,
    created_at: Math.floor(Date.now() / 1000),
    token_used: tokens,
    count: 1,
    quota: credits,
    quota_usd: (credits / 500_000).toFixed(6),
    quota_money_available: true,
    quota_money_basis: 'captured_credits_per_usd',
    output_image_count: images,
    token_count_scope: 'settled_text_requests',
    usage_scope: 'actor_personal_usage',
    settlement_scope: 'settled_requests',
    platform_sponsored: sponsored,
    payer_kind: 'personal',
    subscription_funded: false,
    gross_settled_credits: credits + 250_000,
    refunded_credits: 250_000,
    net_consumption_credits: credits,
    billing_basis: 'net_consumption',
    billing_currency: 'USD',
    wallet_debit_supported: false,
  }
}

async function inspectNativeReport(
  rows: UsageBucket[],
  inspect: (report: { host: HTMLDivElement; markdown: string }) => Promise<void>
) {
  const previousAuth = useAuthStore.getState().auth
  const previousCurrency = useSystemConfigStore.getState().config.currency
  const previousGet = api.get
  const previousLanguage = i18n.language
  const previousClipboard = await navigator.clipboard.readText()
  let calls = 0
  let markdown = ''
  useAuthStore.getState().auth.setUser({
    id: 42,
    username: 'native-report-owner',
    role: 1,
    setting: { wallet_display_currency: 'USD' },
  })
  useSystemConfigStore.getState().setConfig({
    currency: {
      ...DEFAULT_CURRENCY_CONFIG,
      currencyUnit: 'credit',
      creditsPerUsd: 500_000,
      creditsPerUsdExact: '500000',
    },
  })
  await i18n.changeLanguage('en')
  api.get = (async () => {
    calls += 1
    return { data: { success: true, message: '', data: rows } }
  }) as typeof api.get
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const host = document.createElement('div')
  document.body.append(host)
  const root = createRoot(host)
  const route = createRootRoute({
    component: () => (
      <QueryClientProvider client={client}>
        <I18nextProvider i18n={i18n}>
          <ThemeProvider defaultTheme='light'>
            <ModelUsageReport
              rangeKey='7d'
              onCopySnapshotChange={(snapshot) => {
                markdown = snapshot?.markdown ?? ''
              }}
            />
          </ThemeProvider>
        </I18nextProvider>
      </QueryClientProvider>
    ),
  })
  const router = createRouter({
    routeTree: route,
    history: createMemoryHistory({ initialEntries: ['/'] }),
  })
  try {
    await act(async () => root.render(<RouterProvider router={router} />))
    for (let attempt = 0; !markdown && attempt < 30; attempt += 1) {
      await act(async () => {
        await new Promise((resolve) => setTimeout(resolve, 5))
      })
    }
    assert.ok(markdown, 'the native copied report did not become available')
    const copyButton = host.querySelector<HTMLButtonElement>(
      'button[aria-label="Copy Statistics"]'
    )
    assert.ok(copyButton, 'the native report copy button was not rendered')
    await act(async () => copyButton.click())
    assert.equal(await navigator.clipboard.readText(), markdown)
    assert.equal(calls, 1)
    await inspect({ host, markdown })
  } finally {
    await act(async () => root.unmount())
    client.clear()
    host.remove()
    api.get = previousGet
    useAuthStore.setState({ auth: previousAuth })
    useSystemConfigStore.getState().setConfig({ currency: previousCurrency })
    await i18n.changeLanguage(previousLanguage)
    await navigator.clipboard.writeText(previousClipboard)
  }
}

test('native sponsored images keep missing tokens, image count, settled money and scope in the copied report', async () => {
  await inspectNativeReport(
    [
      nativeBucket({
        modelName: 'image-model',
        tokens: null,
        images: 2,
        sponsored: true,
        credits: 1_250_000,
      }),
    ],
    async ({ host, markdown }) => {
      const totals = new Map(
        [...host.querySelectorAll('dl > div')].map((metric) => [
          metric.querySelector('dt')?.textContent,
          metric.querySelector('dd')?.textContent,
        ])
      )
      assert.equal(totals.get('Total tokens'), 'Not recorded')
      assert.equal(totals.get('Images (billing quantity)'), '2')
      assert.equal(totals.get('Platform-sponsored usage'), '2.5 USD')
      assert.equal(totals.get('Total Usage'), '2.5 USD')
      assert.match(markdown, /\*\*Total tokens\*\*: Not recorded/)
      assert.match(markdown, /\*\*Images \(billing quantity\)\*\*: 2/)
      assert.match(markdown, /\*\*Platform-sponsored usage\*\*: 2\.5 USD/)
      assert.match(markdown, /\*\*Total Usage\*\*: 2\.5 USD/)
      assert.match(
        markdown,
        /\| image-model \| Not recorded \| 2 \| 1 \| 2\.5 USD \| 2\.5 USD \| 100% \|/
      )
      assert.match(markdown, /Tokens cover settled text requests only\./)
      assert.match(
        host.textContent ?? '',
        /Tokens cover settled text requests only\./
      )
      assert.doesNotMatch(
        markdown,
        /\*\*Total tokens\*\*: 0|\| image-model \| 0 \|/
      )
      assert.doesNotMatch(markdown, /wallet|debit/i)
    }
  )
})

test('a mixed native report preserves real zero text tokens and separates sponsored image consumption', async () => {
  await inspectNativeReport(
    [
      nativeBucket({
        modelName: 'image-model',
        tokens: null,
        images: 2,
        sponsored: true,
        credits: 1_250_000,
      }),
      nativeBucket({
        modelName: 'text-zero',
        tokens: 0,
        images: 0,
        sponsored: false,
        credits: 250_000,
      }),
    ],
    async ({ host, markdown }) => {
      const totals = new Map(
        [...host.querySelectorAll('dl > div')].map((metric) => [
          metric.querySelector('dt')?.textContent,
          metric.querySelector('dd')?.textContent,
        ])
      )
      assert.equal(totals.get('Total tokens'), '0')
      assert.equal(totals.get('Images (billing quantity)'), '2')
      assert.equal(totals.get('Total requests'), '2')
      assert.equal(totals.get('Platform-sponsored usage'), '2.5 USD')
      assert.equal(totals.get('Total Usage'), '3 USD')
      assert.match(markdown, /\*\*Total tokens\*\*: 0/)
      assert.match(
        markdown,
        /\| image-model \| Not recorded \| 2 \| 1 \| 2\.5 USD \| 2\.5 USD \|/
      )
      assert.match(
        markdown,
        /\| text-zero \| 0 \| 0 \| 1 \| 0\.5 USD \| 0 USD \|/
      )
      assert.match(markdown, /\*\*Platform-sponsored usage\*\*: 2\.5 USD/)
      assert.match(markdown, /\*\*Total Usage\*\*: 3 USD/)
      assert.match(markdown, /Tokens cover settled text requests only\./)
      assert.doesNotMatch(
        markdown,
        /\*\*Total tokens\*\*: Not recorded|wallet|debit/i
      )
    }
  )
})
