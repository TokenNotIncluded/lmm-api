/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.
*/
import assert from 'node:assert/strict'
import { after, afterEach, describe, test } from 'node:test'

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

const { act, createElement } = await import('react')
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
const { UnifiedTodoList } = await import('./unified-todo-list')

const originalGet = api.get
const originalPost = api.post
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
  await new Promise((resolve) => setTimeout(resolve, 25))
}

async function renderList() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const rootRoute = createRootRoute({
    component: () =>
      createElement(
        QueryClientProvider,
        { client: queryClient },
        createElement(I18nextProvider, { i18n }, createElement(UnifiedTodoList))
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
    root.render(createElement(RouterProvider, { router }))
    await flushEffects()
  })
  await act(flushEffects)
  return { container, queryClient, root }
}

async function unmount(rendered: Awaited<ReturnType<typeof renderList>>) {
  await act(async () => rendered.root.unmount())
  rendered.queryClient.clear()
  rendered.container.remove()
}

afterEach(() => {
  api.get = originalGet
  api.post = originalPost
  document.body.replaceChildren()
})

after(() => domWindow.close())

describe('UnifiedTodoList interaction', () => {
  test('marks a notification as read even when it has no destination', async () => {
    const posts: Array<{ url: string; body: unknown }> = []
    api.get = (async () => ({
      data: {
        success: true,
        data: {
          items: [
            {
              id: 'open_source_bounty:12',
              source_id: 12,
              category: 'open_source_bounty',
              type: 'comment',
              title: 'Bounty update',
              summary: 'A new comment needs your attention.',
              read: false,
              created_at: 1_786_400_000,
              updated_at: 1_786_400_000,
            },
          ],
          page: 1,
          page_size: 50,
          total: 1,
          category: 'all',
          unread_count: 1,
          total_unread_count: 1,
          unread_by_category: { open_source_bounty: 1 },
          categories: [{ key: 'open_source_bounty', total: 1, unread: 1 }],
        },
      },
    })) as typeof api.get
    api.post = (async (url: string, body: unknown) => {
      posts.push({ url, body })
      return { data: { success: true, data: { marked: 1 } } }
    }) as typeof api.post

    const rendered = await renderList()
    try {
      const categoryButtons = [...rendered.container.querySelectorAll('button')]
      const allCategory = categoryButtons.find((candidate) =>
        candidate.textContent?.startsWith('All')
      )
      const bountyCategory = categoryButtons.find((candidate) =>
        candidate.textContent?.startsWith('Bounty notifications')
      )
      assert.equal(allCategory?.getAttribute('aria-pressed'), 'true')
      assert.equal(bountyCategory?.getAttribute('aria-pressed'), 'false')
      assert.ok(allCategory?.classList.contains('min-h-11'))

      const row = [...rendered.container.querySelectorAll('button')].find(
        (candidate) =>
          candidate.textContent?.includes('A new comment needs your attention.')
      )
      assert.ok(row)

      await act(async () => {
        row.click()
        await flushEffects()
      })
      await act(flushEffects)

      assert.deepEqual(posts, [
        {
          url: '/api/todos/read',
          body: {
            category: 'open_source_bounty',
            ids: [12],
            all: false,
          },
        },
      ])
    } finally {
      await unmount(rendered)
    }
  })
})
