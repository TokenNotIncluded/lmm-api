/*
Copyright (C) 2026 LIghtJUNction
*/
import assert from 'node:assert/strict'
import { after, afterEach, describe, test } from 'node:test'

import { Window } from 'happy-dom'

const domWindow = new Window({
  url: 'https://console.example.test/drawing',
  width: 390,
  height: 844,
})
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
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
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { api } = await import('@/lib/api')
const { Drawing } = await import('./index')

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

const pricing = {
  success: true,
  data: [
    {
      id: 1,
      model_name: 'image-2',
      quota_type: 1 as const,
      model_ratio: 1,
      completion_ratio: 1,
      enable_groups: ['mobile-image-group'],
      supported_endpoint_types: ['image-generation'],
    },
  ],
  vendors: [],
  group_ratio: {},
  usable_group: {
    'mobile-image-group': {
      desc: 'A long routing description that must stay inside a 390 pixel mobile control.',
      ratio: 1,
    },
  },
  supported_endpoint: {},
  auto_groups: [],
}

async function flushEffects() {
  await new Promise((resolve) => setTimeout(resolve, 20))
}

async function waitForCondition(
  condition: () => boolean,
  failureMessage: string
) {
  for (let attempt = 0; attempt < 80; attempt += 1) {
    if (condition()) return
    await flushEffects()
  }
  throw new Error(failureMessage)
}

async function renderDrawing() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  await act(async () => {
    root.render(
      <QueryClientProvider client={queryClient}>
        <I18nextProvider i18n={i18n}>
          <Drawing />
        </I18nextProvider>
      </QueryClientProvider>
    )
    await flushEffects()
  })
  await act(flushEffects)
  return { container, queryClient, root }
}

afterEach(() => {
  api.get = originalGet
  document.body.replaceChildren()
})

after(() => domWindow.close())

describe('Drawing mobile controls', () => {
  test('keeps every native select full width with long group descriptions', async () => {
    api.get = (async (url: string) => {
      if (url === '/api/assistant/status') {
        return {
          data: {
            success: true,
            data: {
              enabled: true,
              model: 'assistant-test',
              developer_access_granted: true,
              funding: { mode: 'super_administrator' },
            },
          },
        }
      }
      if (url === '/api/pricing') return { data: pricing }
      if (url === '/api/user/self/groups') {
        return {
          data: {
            success: true,
            data: pricing.usable_group,
          },
        }
      }
      throw new Error(`unexpected GET ${url}`)
    }) as typeof api.get

    const rendered = await renderDrawing()
    try {
      await act(
        async () =>
          await waitForCondition(
            () => rendered.container.querySelectorAll('select').length === 5,
            'drawing controls did not render'
          )
      )

      const selects = rendered.container.querySelectorAll('select')
      assert.equal(selects.length, 5)
      for (const select of selects) {
        const wrapper = select.closest<HTMLElement>(
          '[data-slot="native-select-wrapper"]'
        )
        assert.ok(wrapper)
        assert.match(wrapper.className, /\bw-full\b/)
        assert.doesNotMatch(wrapper.className, /\bw-fit\b/)
      }
    } finally {
      await act(async () => rendered.root.unmount())
      rendered.queryClient.clear()
    }
  })
})
