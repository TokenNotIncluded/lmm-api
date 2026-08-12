import assert from 'node:assert/strict'
import { after, afterEach, describe, test } from 'node:test'

import { Window } from 'happy-dom'

const domWindow = new Window({ url: 'https://console.example.test/' })
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'HTMLButtonElement',
  'HTMLInputElement',
  'HTMLTextAreaElement',
  'SVGElement',
  'Node',
  'Element',
  'Event',
  'MouseEvent',
  'PointerEvent',
  'FocusEvent',
  'CustomEvent',
  'MutationObserver',
  'ResizeObserver',
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
const { QueryClient, QueryClientProvider } =
  await import('@tanstack/react-query')
const { api } = await import('@/lib/api')
const { AssistantUserProfileEditor } =
  await import('./assistant-user-profile-editor')

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

async function flushEffects() {
  await new Promise((resolve) => setTimeout(resolve, 20))
}

afterEach(() => {
  api.get = originalGet
  document.body.replaceChildren()
})

after(() => domWindow.close())

describe('AssistantUserProfileEditor', () => {
  test('renders internal profile controls without exposing them to ordinary users', async () => {
    api.get = (async (url: string) => {
      assert.equal(url, '/api/user/41/assistant-profile')
      return {
        data: {
          success: true,
          data: {
            profile_key: 'guided_buyer',
            tags: ['new-user'],
            strategy: 'Ask one question at a time.',
            enabled: true,
            updated_at: 1,
          },
        },
      }
    }) as typeof api.get

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
            <AssistantUserProfileEditor userId={41} open />
          </I18nextProvider>
        </QueryClientProvider>
      )
      await flushEffects()
    })

    try {
      for (let attempt = 0; attempt < 30; attempt += 1) {
        if (
          document.body.textContent?.includes('Ask one question at a time.')
        ) {
          break
        }
        await act(flushEffects)
      }
      assert.match(document.body.textContent ?? '', /Assistant user profile/)
      assert.match(
        document.body.textContent ?? '',
        /Internal moderation guidance/
      )
      assert.match(
        document.body.textContent ?? '',
        /Ask one question at a time\./
      )
      assert.match(document.body.textContent ?? '', /must not contain secrets/)
    } finally {
      await act(async () => root.unmount())
      queryClient.clear()
      container.remove()
    }
  })
})
