/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import assert from 'node:assert/strict'
import { after, test } from 'node:test'

import { Window } from 'happy-dom'

import type { UserProfile } from '../types'

const dom = new Window({ url: 'https://console.example.test/profile' })
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
const { ProfileActivity } = await import('./profile-activity')
const { ProfileHeader } = await import('./profile-header')
const { ProfileShareDialog } = await import('./profile-share-dialog')
const { buildProfileDailyUsage, buildProfileUsageSummary } =
  await import('../lib/activity')
const { drawProfileShareCard } = await import('../lib/share-card')
const { api } = await import('@/lib/api')
const { useAuthStore } = await import('@/stores/auth-store')
const i18n = createInstance()
await i18n.use(initReactI18next).init({
  lng: 'en',
  resources: { en: { translation: {} } },
})
after(() => dom.close())

const profile: UserProfile = {
  id: 7,
  username: 'alice',
  display_name: 'Alice',
  role: 1,
  group: 'default',
  quota: 0,
  used_quota: 0,
  request_count: 1,
  status: 1,
  aff_count: 0,
  aff_quota: 0,
  aff_history_quota: 0,
  created_time: Math.floor(Date.now() / 1000) - 86_400,
}

function canvasContext(renderedText: string[]) {
  return {
    save() {},
    restore() {},
    beginPath() {},
    arc() {},
    clip() {},
    fillRect() {},
    roundRect() {},
    stroke() {},
    moveTo() {},
    lineTo() {},
    fill() {},
    fillText(value: string) {
      renderedText.push(value)
    },
    measureText(value: string) {
      return { width: value.length * 8 }
    },
  }
}

test('activity tooltips retain image-only null tokens, real text zero, images and sponsorship', async () => {
  const imageDate = new Date(2026, 9, 20, 12)
  const textDate = new Date(2026, 9, 21, 12)
  const days = buildProfileDailyUsage(
    [
      {
        created_at: imageDate.getTime() / 1000,
        token_used: null,
        count: 1,
        output_image_count: 2,
        token_count_scope: 'settled_text_requests',
        platform_sponsored: true,
      },
      {
        created_at: textDate.getTime() / 1000,
        token_used: 0,
        count: 1,
        output_image_count: 0,
        token_count_scope: 'settled_text_requests',
        platform_sponsored: false,
      },
    ],
    { startDate: imageDate, endDate: textDate }
  )
  const host = document.createElement('div')
  document.body.append(host)
  const root = createRoot(host)
  try {
    await act(async () => {
      root.render(
        <I18nextProvider i18n={i18n}>
          <ProfileActivity
            days={days}
            activeDays={0}
            loading={false}
            error={null}
            onRetry={() => undefined}
          />
        </I18nextProvider>
      )
    })
    const titles = [...host.querySelectorAll('span[title]')].map(
      (element) => element.getAttribute('title') ?? ''
    )
    assert.equal(titles.length, 2)
    assert.match(titles[0], /Not recorded tokens/)
    assert.doesNotMatch(titles[0], /(?:^|\s)0 tokens/)
    assert.match(titles[0], /2 Images/)
    assert.match(titles[0], /1 Platform sponsored requests/)
    assert.match(titles[1], /0 tokens/)
    assert.match(titles[1], /0 Images/)
    assert.match(titles[1], /0 Platform sponsored requests/)
    assert.ok(
      titles.every((title) =>
        title.includes('Tokens cover settled text requests only.')
      )
    )
    assert.match(
      host.textContent ?? '',
      /Tokens cover settled text requests only\./
    )
    assert.doesNotMatch(host.textContent ?? '', /wallet|debit/i)
  } finally {
    await act(async () => root.unmount())
    host.remove()
  }
})

test('the share PNG canvas includes unavailable tokens, images and text-only scope', () => {
  const canvas = document.createElement('canvas')
  const renderedText: string[] = []
  Object.defineProperty(canvas, 'getContext', {
    value: () => canvasContext(renderedText),
  })
  drawProfileShareCard(canvas, {
    displayName: 'Alice',
    username: 'alice',
    roleLabel: 'User',
    totalTokens: 'Unavailable',
    peakDailyTokens: 'Unavailable',
    requestCount: '1',
    currentStreak: '0 days',
    longestStreak: '0 days',
    imageCount: '2',
    sponsoredRequests: '1',
    tokenScopeNote: 'Tokens cover settled text requests only.',
    days: [],
    activityAvailable: true,
    locale: 'en',
    labels: {
      totalTokens: 'Tokens in the past year',
      activity: 'Token activity',
      activityUnavailable: 'Activity unavailable',
      peakDailyTokens: 'Peak daily tokens',
      requestCount: 'API Requests',
      currentStreak: 'Current streak',
      longestStreak: 'Longest streak this year',
      imageCount: 'Images',
      sponsoredRequests: 'Platform sponsored requests',
    },
  })
  assert.equal(renderedText.filter((text) => text === 'Unavailable').length, 2)
  assert.ok(renderedText.includes('Images: 2 · Platform sponsored requests: 1'))
  assert.ok(renderedText.includes('Tokens cover settled text requests only.'))
  assert.doesNotMatch(renderedText.join('\n'), /wallet|debit/i)
})

test('profile header shows unavailable image tokens and keeps actual zero text tokens', async () => {
  const previousGet = api.get
  const previousAuth = useAuthStore.getState().auth
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const host = document.createElement('div')
  document.body.append(host)
  const root = createRoot(host)
  let tokenUsed: number | null = null
  api.get = (async () => ({
    data: {
      success: true,
      data: [
        {
          user_id: profile.id,
          model_name: tokenUsed === null ? 'image-model' : 'text-model',
          created_at: Math.floor(Date.now() / 1000),
          token_used: tokenUsed,
          count: 1,
          quota: 0,
          quota_usd: '0.000000',
          quota_money_available: true,
          quota_money_basis: 'captured_credits_per_usd',
          output_image_count: tokenUsed === null ? 2 : 0,
          token_count_scope: 'settled_text_requests',
          usage_scope: 'actor_personal_usage',
          settlement_scope: 'settled_requests',
          platform_sponsored: tokenUsed === null,
          payer_kind: 'personal',
          subscription_funded: false,
          gross_settled_credits: 0,
          refunded_credits: 0,
          net_consumption_credits: 0,
          billing_basis: 'net_consumption',
          billing_currency: 'USD',
          wallet_debit_supported: false,
        },
      ],
    },
  })) as typeof api.get
  useAuthStore.getState().auth.setUser({
    id: profile.id,
    username: profile.username,
    role: profile.role,
  })
  const route = createRootRoute({
    component: () => (
      <QueryClientProvider client={client}>
        <I18nextProvider i18n={i18n}>
          <ProfileHeader profile={profile} loading={false} />
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
    for (
      let attempt = 0;
      !host.textContent?.includes('Images: 2') && attempt < 30;
      attempt += 1
    ) {
      await act(async () => {
        await new Promise((resolve) => setTimeout(resolve, 5))
      })
    }
    assert.equal(host.querySelectorAll('dd')[0]?.textContent, 'Unavailable')
    assert.equal(host.querySelectorAll('dd')[1]?.textContent, 'Unavailable')
    assert.match(
      host.textContent ?? '',
      /Images: 2 · Platform sponsored requests: 1/
    )
    assert.match(
      host.textContent ?? '',
      /Tokens cover settled text requests only\./
    )
    tokenUsed = 0
    await act(async () => {
      await client.invalidateQueries()
    })
    for (
      let attempt = 0;
      host.querySelectorAll('dd')[0]?.textContent !== '0' && attempt < 30;
      attempt += 1
    ) {
      await act(async () => {
        await new Promise((resolve) => setTimeout(resolve, 5))
      })
    }
    assert.equal(host.querySelectorAll('dd')[0]?.textContent, '0')
    assert.equal(host.querySelectorAll('dd')[1]?.textContent, '0')
  } finally {
    await act(async () => root.unmount())
    client.clear()
    api.get = previousGet
    useAuthStore.setState({ auth: previousAuth })
    host.remove()
  }
})

test('the share dialog exports image token null as unavailable and text zero as zero', async () => {
  const renderedText: string[] = []
  const prototype = dom.HTMLCanvasElement.prototype
  const previousContext = Object.getOwnPropertyDescriptor(
    prototype,
    'getContext'
  )
  const previousBlob = Object.getOwnPropertyDescriptor(prototype, 'toBlob')
  Object.defineProperty(prototype, 'getContext', {
    configurable: true,
    value: () => canvasContext(renderedText),
  })
  Object.defineProperty(prototype, 'toBlob', {
    configurable: true,
    value: (callback: BlobCallback) =>
      callback(new Blob(['png'], { type: 'image/png' })),
  })
  const host = document.createElement('div')
  document.body.append(host)
  const root = createRoot(host)
  const renderDialog = async (tokens: number | null) => {
    const now = new Date()
    const days = buildProfileDailyUsage(
      [
        {
          created_at: now.getTime() / 1000,
          count: 1,
          token_used: tokens,
          output_image_count: tokens === null ? 2 : 0,
          platform_sponsored: tokens === null,
          token_count_scope: 'settled_text_requests',
        },
      ],
      { startDate: now, endDate: now }
    )
    renderedText.length = 0
    await act(async () => {
      root.render(
        <I18nextProvider i18n={i18n}>
          <ProfileShareDialog
            open
            onOpenChange={() => undefined}
            profile={profile}
            days={days}
            summary={buildProfileUsageSummary(days)}
            activityAvailable
          />
        </I18nextProvider>
      )
    })
    for (
      let attempt = 0;
      renderedText.length === 0 && attempt < 30;
      attempt += 1
    ) {
      await act(async () => {
        await new Promise((resolve) => setTimeout(resolve, 5))
      })
    }
  }
  try {
    await renderDialog(null)
    assert.equal(
      renderedText.filter((text) => text === 'Unavailable').length,
      2
    )
    assert.ok(
      renderedText.includes('Images: 2 · Platform sponsored requests: 1')
    )
    assert.ok(renderedText.includes('Tokens cover settled text requests only.'))
    await renderDialog(0)
    assert.equal(renderedText.filter((text) => text === '0').length, 2)
    assert.ok(!renderedText.includes('Unavailable'))
    assert.ok(
      renderedText.includes('Images: 0 · Platform sponsored requests: 0')
    )
    assert.doesNotMatch(renderedText.join('\n'), /wallet|debit/i)
  } finally {
    await act(async () => root.unmount())
    if (previousContext) {
      Object.defineProperty(prototype, 'getContext', previousContext)
    } else {
      Reflect.deleteProperty(prototype, 'getContext')
    }
    if (previousBlob) {
      Object.defineProperty(prototype, 'toBlob', previousBlob)
    } else {
      Reflect.deleteProperty(prototype, 'toBlob')
    }
    host.remove()
  }
})
