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
*/
import assert from 'node:assert/strict'
import { after, afterEach, describe, test } from 'node:test'

import { Window } from 'happy-dom'

import type { AuthUser } from '@/stores/auth-store'

const domWindow = new Window({ url: 'https://console.example.test/' })
for (const key of [
  'window',
  'document',
  'navigator',
  'history',
  'location',
  'HTMLElement',
  'HTMLAnchorElement',
  'HTMLButtonElement',
  'HTMLFormElement',
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
  Outlet,
  RouterProvider,
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
} = await import('@tanstack/react-router')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { api } = await import('@/lib/api')
const { consumeQueuedAssistantRequest, subscribeToAssistantOpen } =
  await import('@/features/assistant/assistant-events')
const { useAuthStore } = await import('@/stores/auth-store')
const { ForgeHome } = await import('./forge-home')

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

function makeRouter() {
  const rootRoute = createRootRoute({ component: Outlet })
  const homeRoute = createRoute({
    getParentRoute: () => rootRoute,
    path: '/',
    component: ForgeHome,
  })
  const routes = [
    '/about',
    '/challenges',
    '/dashboard',
    '/getting-started',
    '/open-source-bounties',
    '/pricing',
    '/security',
    '/sign-in',
  ].map((path) =>
    createRoute({
      getParentRoute: () => rootRoute,
      path,
      component: () => null,
    })
  )
  const challengeDetailRoute = createRoute({
    getParentRoute: () => rootRoute,
    path: '/challenges/$challengeId',
    component: () => null,
  })

  return createRouter({
    routeTree: rootRoute.addChildren([
      homeRoute,
      ...routes,
      challengeDetailRoute,
    ]),
    history: createMemoryHistory({ initialEntries: ['/'] }),
  })
}

async function renderHome(user: AuthUser | null) {
  useAuthStore.getState().auth.setUser(user)
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: {
        refetchOnReconnect: false,
        refetchOnWindowFocus: false,
        retry: false,
      },
    },
  })
  const router = makeRouter()
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)

  api.get = (async (url) => {
    if (url === '/api/notice') {
      return { data: { success: true, data: '' } }
    }
    if (url === '/api/status') {
      return {
        data: {
          success: true,
          data: {
            backend_capabilities: { bounty_public_read: false },
          },
        },
      }
    }
    return { data: { success: true, data: [] } }
  }) as typeof api.get

  await act(async () => {
    root.render(
      <QueryClientProvider client={queryClient}>
        <I18nextProvider i18n={i18n}>
          <RouterProvider router={router} />
        </I18nextProvider>
      </QueryClientProvider>
    )
    await flushEffects()
  })
  await act(flushEffects)

  return { container, queryClient, root, router }
}

async function unmountHome(rendered: Awaited<ReturnType<typeof renderHome>>) {
  await act(async () => rendered.root.unmount())
  rendered.queryClient.clear()
  rendered.container.remove()
}

function findMessageInput(container: HTMLElement) {
  const input = container.querySelector<HTMLInputElement>('#forge-home-message')
  assert.ok(input)
  return input
}

async function submitMessage(container: HTMLElement, message: string) {
  const input = findMessageInput(container)
  const form = container.querySelector<HTMLFormElement>('form')
  assert.ok(form)
  const setValue = Object.getOwnPropertyDescriptor(
    HTMLInputElement.prototype,
    'value'
  )?.set
  assert.ok(setValue)

  await act(async () => {
    setValue.call(input, message)
    input.dispatchEvent(new Event('input', { bubbles: true }))
    await flushEffects()
  })
  await act(async () => {
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }))
    await flushEffects()
  })
}

afterEach(() => {
  consumeQueuedAssistantRequest()
  api.get = originalGet
  useAuthStore.getState().auth.reset('complete')
  window.localStorage.clear()
  window.sessionStorage.clear()
  document.body.replaceChildren()
})

after(() => domWindow.close())

describe('ForgeHome assistant entry', () => {
  test('queues onboarding with the message and redirects anonymous users to sign-in', async () => {
    const opened: Array<{
      autoSend: boolean
      message: string | undefined
      preset: string | undefined
    }> = []
    const unsubscribe = subscribeToAssistantOpen((queued) => {
      opened.push({
        preset: queued.preset,
        message: queued.message,
        autoSend: queued.autoSend,
      })
    })
    const rendered = await renderHome(null)

    await submitMessage(rendered.container, '  Help me configure the SDK  ')

    assert.deepEqual(opened, [
      {
        preset: undefined,
        message: 'Help me configure the SDK',
        autoSend: true,
      },
    ])
    assert.equal(rendered.router.state.location.pathname, '/sign-in')
    assert.deepEqual(rendered.router.state.location.search, {
      redirect: '/dashboard',
    })

    unsubscribe()
    await unmountHome(rendered)
  })

  test('queues onboarding with the message and redirects an L0 user to getting started', async () => {
    const opened: Array<{
      autoSend: boolean
      message: string | undefined
      preset: string | undefined
    }> = []
    const unsubscribe = subscribeToAssistantOpen((queued) => {
      opened.push({
        preset: queued.preset,
        message: queued.message,
        autoSend: queued.autoSend,
      })
    })
    const rendered = await renderHome({
      id: 7,
      username: 'l0-user',
      role: 1,
      developer_access_granted: false,
    })

    await submitMessage(rendered.container, '  I need L1 access  ')

    assert.deepEqual(opened, [
      { preset: 'onboarding', message: 'I need L1 access', autoSend: true },
    ])
    assert.equal(rendered.router.state.location.pathname, '/getting-started')

    unsubscribe()
    await unmountHome(rendered)
  })

  test('queues service guidance with the message and redirects an L1 user to the dashboard', async () => {
    const opened: Array<{
      autoSend: boolean
      message: string | undefined
      preset: string | undefined
    }> = []
    const unsubscribe = subscribeToAssistantOpen((queued) => {
      opened.push({
        preset: queued.preset,
        message: queued.message,
        autoSend: queued.autoSend,
      })
    })
    const rendered = await renderHome({
      id: 8,
      username: 'l1-user',
      role: 1,
      developer_access_granted: true,
    })

    await submitMessage(rendered.container, '  Show me the API setup  ')

    assert.deepEqual(opened, [
      { preset: 'service', message: 'Show me the API setup', autoSend: true },
    ])
    assert.equal(rendered.router.state.location.pathname, '/dashboard')

    unsubscribe()
    await unmountHome(rendered)
  })

  test('does not submit whitespace-only messages', async () => {
    const opened: Array<string | undefined> = []
    const unsubscribe = subscribeToAssistantOpen((request) => {
      opened.push(request.preset)
    })
    const rendered = await renderHome(null)

    await submitMessage(rendered.container, '   \n\t  ')

    const submit = rendered.container.querySelector<HTMLButtonElement>(
      'button[type="submit"]'
    )
    assert.ok(submit)
    assert.equal(submit.disabled, true)
    assert.deepEqual(opened, [])
    assert.equal(rendered.router.state.location.pathname, '/')

    unsubscribe()
    await unmountHome(rendered)
  })

  test('redacts sensitive values before queuing them across login', async () => {
    const queued: Array<
      NonNullable<ReturnType<typeof consumeQueuedAssistantRequest>>
    > = []
    const unsubscribe = subscribeToAssistantOpen((request) => {
      queued.push(request)
    })
    const rendered = await renderHome(null)

    await submitMessage(
      rendered.container,
      'Help configure the SDK for alice@example.test with sk-secret1234567890'
    )

    assert.equal(queued.length, 1)
    assert.equal(queued[0]?.autoSend, true)
    assert.equal(
      queued[0]?.message,
      'Help configure the SDK for [REDACTED_EMAIL] with [REDACTED_API_KEY]'
    )
    assert.equal(
      window.sessionStorage.getItem('lmm_assistant_queued_message'),
      null
    )
    const storage = JSON.stringify({ ...window.sessionStorage })
    assert.equal(storage.includes('alice@example.test'), false)
    assert.equal(storage.includes('sk-secret1234567890'), false)

    unsubscribe()
    await unmountHome(rendered)
  })
})
