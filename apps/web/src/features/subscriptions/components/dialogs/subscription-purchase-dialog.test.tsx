/*
Copyright (C) 2026 LIghtJUNction
*/
import assert from 'node:assert/strict'
import { after, afterEach, describe, test } from 'node:test'

import { Window } from 'happy-dom'

import type { PlanRecord } from '../../types'

const domWindow = new Window({
  url: 'https://console.example.test/wallet',
  width: 390,
  height: 844,
})
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'HTMLButtonElement',
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
] as const) {
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: domWindow[key],
  })
}

const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { api } = await import('@/lib/api')
const { SubscriptionPurchaseDialog } =
  await import('./subscription-purchase-dialog')

const originalPost = api.post
const originalOpen = domWindow.open
const reactTestGlobals = globalThis as typeof globalThis & {
  IS_REACT_ACT_ENVIRONMENT?: boolean
}
reactTestGlobals.IS_REACT_ACT_ENVIRONMENT = true

const i18n = createInstance()
await i18n.use(initReactI18next).init({
  lng: 'en',
  resources: { en: { translation: {} } },
})

const plan: PlanRecord = {
  plan: {
    id: 7,
    title: 'Mobile Starter',
    price_amount: 5,
    currency: 'USD',
    duration_unit: 'month',
    duration_value: 1,
    quota_reset_period: 'monthly',
    enabled: true,
    sort_order: 1,
    max_purchase_per_user: 0,
    total_amount: 100,
    stripe_price_id: 'price_mobile_starter',
    allow_balance_pay: true,
    allow_wallet_overflow: true,
  },
}

async function flushEffects() {
  await new Promise((resolve) => setTimeout(resolve, 20))
}

type Rendered = {
  container: HTMLDivElement
  root: ReturnType<typeof createRoot>
}

async function renderDialog(): Promise<Rendered> {
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  await act(async () => {
    root.render(
      <I18nextProvider i18n={i18n}>
        <SubscriptionPurchaseDialog
          open
          onOpenChange={() => undefined}
          plan={plan}
          enableStripe
          userQuota={0}
        />
      </I18nextProvider>
    )
    await flushEffects()
  })
  return { container, root }
}

async function unmount(rendered: Rendered) {
  await act(async () => rendered.root.unmount())
  rendered.container.remove()
}

afterEach(() => {
  api.post = originalPost
  domWindow.open = originalOpen
  document.body.replaceChildren()
})

after(() => domWindow.close())

describe('subscription purchase checkout', () => {
  test('reserves the Stripe checkout before the async request on mobile', async () => {
    const events: string[] = []
    const popup = {
      closed: false,
      opener: {} as Window | null,
      close: () => undefined,
      focus: () => undefined,
      location: { href: '' },
    }
    const openCalls: Array<[string, string, string]> = []
    domWindow.open = ((url: string, target: string, features: string) => {
      events.push('reserve')
      openCalls.push([url, target, features])
      return popup as unknown as Window
    }) as typeof domWindow.open
    api.post = (async (url) => {
      events.push(`request:${url}`)
      return {
        data: {
          success: true,
          message: 'success',
          data: { pay_link: 'https://pay.example.test/stripe' },
        },
      }
    }) as typeof api.post

    const rendered = await renderDialog()
    try {
      const stripeButton = [...document.querySelectorAll('button')].find(
        (button) => button.textContent?.trim() === 'Stripe'
      )
      assert.ok(stripeButton)

      await act(async () => {
        stripeButton.click()
        await flushEffects()
      })

      assert.deepEqual(events, [
        'reserve',
        'request:/api/subscription/stripe/pay',
      ])
      assert.equal(openCalls.length, 1)
      assert.equal(openCalls[0]?.[0], 'about:blank')
      assert.match(openCalls[0]?.[1] ?? '', /^payment_checkout_/)
      assert.equal(openCalls[0]?.[2], 'noopener,noreferrer')
      assert.equal(popup.location.href, 'https://pay.example.test/stripe')
      assert.equal(popup.opener, null)
    } finally {
      await unmount(rendered)
    }
  })
})
