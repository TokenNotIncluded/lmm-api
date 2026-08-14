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
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { api } = await import('@/lib/api')
const { AssistantUserTodo } = await import('./assistant-user-todo')

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

const pendingHandoff = {
  id: 4,
  user_id: 1,
  source: 'handoff',
  intent: 'human_support',
  message: 'The API key page failed at 10:30 UTC.',
  status: 'pending',
  admin_user_id: 0,
  admin_note: '',
  created_at: 1_786_400_000,
  resolved_at: 0,
  username: 'another-user-must-not-leak',
  email: 'another-user@example.com',
}

const resolvedHandoff = {
  ...pendingHandoff,
  status: 'resolved',
  admin_user_id: 9,
  admin_note: 'Please retry after creating a new key on the Keys page.',
  resolved_at: 1_786_400_900,
}

async function flushEffects() {
  await new Promise((resolve) => setTimeout(resolve, 25))
}

async function renderTodo(
  props: {
    onContinueWithAi?: () => void
    onViewRelatedPage?: () => void
  } = {}
) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)

  await act(async () => {
    root.render(
      createElement(
        QueryClientProvider,
        { client: queryClient },
        createElement(
          I18nextProvider,
          { i18n },
          createElement(AssistantUserTodo, props)
        )
      )
    )
    await flushEffects()
  })
  await act(flushEffects)
  return { container, queryClient, root }
}

function findButton(container: HTMLElement, testId: string): HTMLButtonElement {
  const button = container.querySelector<HTMLButtonElement>(
    `[data-testid="${testId}"]`
  )
  assert.ok(button, `Could not find button ${testId}`)
  return button
}

async function unmount(rendered: Awaited<ReturnType<typeof renderTodo>>) {
  await act(async () => rendered.root.unmount())
  rendered.queryClient.clear()
  rendered.container.remove()
}

afterEach(() => {
  api.get = originalGet
  document.body.replaceChildren()
})

after(() => domWindow.close())

describe('AssistantUserTodo', () => {
  test('shows a loading state while the personal request is loading', async () => {
    api.get = (() => new Promise(() => undefined)) as typeof api.get

    const rendered = await renderTodo()
    assert.ok(
      rendered.container.querySelector(
        '[data-testid="assistant-user-todo-loading"]'
      )
    )
    assert.match(rendered.container.textContent ?? '', /Privacy and redaction/)

    await unmount(rendered)
  })

  test('shows an error state and retries the self request', async () => {
    let getCalls = 0
    api.get = (async (url: string) => {
      assert.equal(url, '/api/assistant/handoffs/self')
      getCalls += 1
      if (getCalls === 1) throw new Error('offline')
      return { data: { success: true, data: null } }
    }) as typeof api.get

    const rendered = await renderTodo()
    assert.ok(
      rendered.container.querySelector(
        '[data-testid="assistant-user-todo-error"]'
      )
    )
    assert.match(
      rendered.container.textContent ?? '',
      /Unable to load your support tasks/
    )

    await act(async () => {
      findButton(rendered.container, 'assistant-user-todo-refresh').click()
      await flushEffects()
    })
    await act(flushEffects)
    assert.equal(getCalls, 2)
    assert.ok(
      rendered.container.querySelector(
        '[data-testid="assistant-user-todo-empty"]'
      )
    )

    await unmount(rendered)
  })

  test('shows an empty state and exposes next-step actions', async () => {
    api.get = (async () => ({
      data: { success: true, data: null },
    })) as typeof api.get
    let continueCalls = 0
    let relatedCalls = 0

    const rendered = await renderTodo({
      onContinueWithAi: () => {
        continueCalls += 1
      },
      onViewRelatedPage: () => {
        relatedCalls += 1
      },
    })
    assert.ok(
      rendered.container.querySelector(
        '[data-testid="assistant-user-todo-empty"]'
      )
    )
    findButton(rendered.container, 'assistant-user-todo-continue-ai').click()
    findButton(rendered.container, 'assistant-user-todo-related').click()
    assert.equal(continueCalls, 1)
    assert.equal(relatedCalls, 1)

    await unmount(rendered)
  })

  test('shows only the current user pending request', async () => {
    api.get = (async () => ({
      data: { success: true, data: pendingHandoff },
    })) as typeof api.get

    const rendered = await renderTodo()
    assert.ok(
      rendered.container.querySelector(
        '[data-testid="assistant-user-todo-pending"]'
      )
    )
    assert.match(
      rendered.container.textContent ?? '',
      /Waiting for an administrator/
    )
    assert.match(
      rendered.container.textContent ?? '',
      /The API key page failed/
    )
    assert.doesNotMatch(
      rendered.container.textContent ?? '',
      /another-user-must-not-leak|another-user@example.com/
    )

    await unmount(rendered)
  })

  test('shows the administrator reply for a resolved request', async () => {
    api.get = (async () => ({
      data: { success: true, data: resolvedHandoff },
    })) as typeof api.get

    const rendered = await renderTodo()
    assert.ok(
      rendered.container.querySelector(
        '[data-testid="assistant-user-todo-resolved"]'
      )
    )
    assert.match(rendered.container.textContent ?? '', /Administrator replied/)
    assert.match(
      rendered.container.textContent ?? '',
      /Please retry after creating a new key/
    )
    assert.doesNotMatch(
      rendered.container.textContent ?? '',
      /another-user-must-not-leak|another-user@example.com/
    )

    await unmount(rendered)
  })
})
