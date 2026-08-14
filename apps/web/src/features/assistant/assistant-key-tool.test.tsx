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
import { after, afterEach, describe, test } from 'node:test'

import { Window } from 'happy-dom'

import type { AssistantCreateKeyAction } from './api'

const domWindow = new Window({ url: 'https://console.example.test/' })
for (const key of [
  'window',
  'document',
  'navigator',
  'history',
  'location',
  'HTMLElement',
  'HTMLButtonElement',
  'HTMLInputElement',
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
const { AssistantKeyTool } = await import('./assistant-key-tool')

const originalPost = api.post
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

type RenderedTool = {
  container: HTMLDivElement
  queryClient: InstanceType<typeof QueryClient>
  root: ReturnType<typeof createRoot>
}

async function flushEffects() {
  await new Promise((resolve) => setTimeout(resolve, 20))
}

async function renderTool(
  developerAccessGranted: boolean,
  onContinueSetup = () => {},
  confirmationAction?: AssistantCreateKeyAction,
  autoConfirm = false
): Promise<RenderedTool> {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const rootRoute = createRootRoute({
    component: () => (
      <QueryClientProvider client={queryClient}>
        <I18nextProvider i18n={i18n}>
          <AssistantKeyTool
            baseUrl='https://api.example.test/v1'
            availableModels={['claude-sonnet-4-5']}
            developerAccessGranted={developerAccessGranted}
            confirmationAction={confirmationAction}
            autoConfirm={autoConfirm}
            onContinueSetup={onContinueSetup}
          />
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
    await flushEffects()
  })
  return { container, queryClient, root }
}

function findButton(text: string): HTMLButtonElement {
  const button = [
    ...document.querySelectorAll<HTMLButtonElement>('button'),
  ].find((candidate) => candidate.textContent?.includes(text))
  assert.ok(button, `Could not find button containing ${text}`)
  return button
}

async function unmount(rendered: RenderedTool) {
  await act(async () => rendered.root.unmount())
  rendered.queryClient.clear()
  rendered.container.remove()
}

afterEach(() => {
  api.post = originalPost
  api.get = originalGet
  document.body.replaceChildren()
})

after(() => domWindow.close())

describe('AssistantKeyTool', () => {
  test('explains and exposes connection values to L0 without a creation action', async () => {
    const rendered = await renderTool(false)

    assert.match(rendered.container.textContent ?? '', /Connection details/)
    assert.match(
      rendered.container.textContent ?? '',
      /Base URL tells your client where to connect/
    )
    assert.match(
      rendered.container.textContent ?? '',
      /https:\/\/api\.example\.test\/v1/
    )
    assert.match(rendered.container.textContent ?? '', /<MODEL_ID>/)
    assert.match(
      rendered.container.textContent ?? '',
      /API key creation requires L1/
    )
    assert.equal(rendered.container.querySelector('#assistant-key-name'), null)
    assert.equal(
      [...rendered.container.querySelectorAll('button')].some((button) =>
        button.textContent?.includes('Review key creation')
      ),
      false
    )

    await unmount(rendered)
  })

  test('keeps L1 key creation confirmation-gated and reveals the secret only from a private card', async () => {
    const posted: Array<{ url: string; data: unknown; config: unknown }> = []
    const fetched: Array<{ url: string; config: unknown }> = []
    let continued = 0
    api.post = (async (url: string, data: unknown, config: unknown) => {
      posted.push({ url, data, config })
      assert.equal(url, '/api/assistant/tools/create-key')
      return {
        data: {
          success: true,
          data: {
            id: 9,
            name: 'AI assistant key',
            group: 'auto',
            expired_time: -1,
            card: { id: 'card-9', label: 'Private API key' },
          },
        },
      }
    }) as typeof api.post
    api.get = (async (url: string, config: unknown) => {
      fetched.push({ url, config })
      assert.equal(url, '/api/assistant/cards/card-9/reveal')
      return {
        data: {
          success: true,
          data: { payload: { api_key: 'sk-created-by-test' } },
        },
      }
    }) as typeof api.get
    const rendered = await renderTool(true, () => {
      continued += 1
    })

    await act(async () => {
      findButton('Review key creation').click()
      await flushEffects()
    })
    assert.match(document.body.textContent ?? '', /Create this API key\?/)

    await act(async () => {
      findButton('Confirm and create').click()
      await flushEffects()
    })

    assert.deepEqual(posted[0]?.url, '/api/assistant/tools/create-key')
    assert.deepEqual(posted[0]?.data, {
      confirmed: true,
      name: 'AI assistant key',
      group: 'auto',
    })
    assert.match(rendered.container.textContent ?? '', /API key created/)
    assert.match(rendered.container.textContent ?? '', /claude-sonnet-4-5/)
    assert.match(rendered.container.textContent ?? '', /Private API key/)
    assert.doesNotMatch(
      rendered.container.textContent ?? '',
      /sk-created-by-test/
    )
    assert.ok(
      rendered.container.querySelector('[data-testid="assistant-private-card"]')
    )
    assert.equal(continued, 0)

    let openedUrl = ''
    Object.defineProperty(domWindow, 'open', {
      configurable: true,
      value: (url: string) => {
        openedUrl = url
        return null
      },
    })

    await act(async () => {
      findButton('Import to CC Switch').click()
      await flushEffects()
    })
    assert.equal(
      openedUrl,
      'ccswitch://v1/import?resource=provider&app=claude&name=LMM&endpoint=https%3A%2F%2Fapi.example.test&apiKey=sk-created-by-test&model=claude-sonnet-4-5&homepage=https%3A%2F%2Fapi.example.test&enabled=true'
    )

    await act(async () => {
      findButton('Show securely').click()
      await flushEffects()
    })
    assert.ok(
      fetched.some(({ url }) => url === '/api/assistant/cards/card-9/reveal')
    )
    assert.match(rendered.container.textContent ?? '', /sk-created-by-test/)

    await act(async () => {
      findButton('Hide credential').click()
      await flushEffects()
    })
    assert.doesNotMatch(
      rendered.container.textContent ?? '',
      /sk-created-by-test/
    )

    await act(async () => {
      findButton('I copied it — continue setup').click()
      await flushEffects()
    })
    assert.equal(continued, 1)

    await unmount(rendered)
  })

  test('uses the assistant preview token and server-owned name/group', async () => {
    let posted: unknown
    api.post = (async (_url: string, data: unknown) => {
      posted = data
      return {
        data: {
          success: true,
          data: {
            id: 10,
            name: 'test',
            group: 'GPT-Auto',
            expired_time: -1,
            card: { id: 'card-10', label: 'Private API key' },
          },
        },
      }
    }) as typeof api.post
    const rendered = await renderTool(true, () => {}, {
      type: 'create_key',
      confirmation_token: 'preview-token',
      requires_confirmation: true,
      expires_in_seconds: 600,
      name: 'test',
      group: 'GPT-Auto',
    })

    await act(async () => {
      findButton('Review key creation').click()
      await flushEffects()
      findButton('Confirm and create').click()
      await flushEffects()
    })
    assert.deepEqual(posted, {
      confirmed: true,
      name: 'test',
      group: 'GPT-Auto',
      confirmation_token: 'preview-token',
    })
    await unmount(rendered)
  })

  test('confirms an affirmative chat reply through the pending action', async () => {
    let posted: unknown
    api.post = (async (_url: string, data: unknown) => {
      posted = data
      return {
        data: {
          success: true,
          data: {
            id: 11,
            name: 'chat-created',
            group: 'GPT-Pro',
            expired_time: -1,
            card: { id: 'card-11', label: 'Private API key' },
          },
        },
      }
    }) as typeof api.post
    const rendered = await renderTool(
      true,
      () => {},
      {
        type: 'create_key',
        confirmation_token: 'chat-token',
        requires_confirmation: true,
        expires_in_seconds: 600,
        name: 'chat-created',
        group: 'GPT-Pro',
      },
      true
    )

    await act(async () => {
      await flushEffects()
    })
    assert.deepEqual(posted, {
      confirmed: true,
      name: 'chat-created',
      group: 'GPT-Pro',
      confirmation_token: 'chat-token',
    })
    assert.match(rendered.container.textContent ?? '', /API key created/)
    assert.ok(
      rendered.container.querySelector('[data-testid="assistant-private-card"]')
    )
    assert.doesNotMatch(rendered.container.textContent ?? '', /sk-/)

    await unmount(rendered)
  })
})
