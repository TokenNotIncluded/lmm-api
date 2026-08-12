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
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { api } = await import('@/lib/api')
const { AssistantHistory } = await import('./assistant-history')

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

const activeConversation = {
  id: 1,
  title: 'Active support',
  last_message_preview: 'active-support api_key=sk-history-secret-123456',
  created_at: 1_786_400_000,
  updated_at: 1_786_400_001,
  archived_at: 0,
  owner: 'self' as const,
  privacy_notice: 'Conversations are not private.',
}

const lowerAccessConversation = {
  ...activeConversation,
  id: 2,
  last_message_preview: 'lower-access-support',
  owner: 'lower_level_user' as const,
}

const archivedConversation = {
  ...activeConversation,
  id: 3,
  last_message_preview: 'archived-support',
  archived_at: 1_786_400_100,
}

async function flushEffects() {
  await new Promise((resolve) => setTimeout(resolve, 25))
}

async function renderHistory() {
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
          <AssistantHistory active onOpenConversation={() => {}} />
        </I18nextProvider>
      </QueryClientProvider>
    )
    await flushEffects()
  })
  await act(flushEffects)
  return { container, queryClient, root }
}

function findButton(text: string): HTMLButtonElement {
  const button = [
    ...document.querySelectorAll<HTMLButtonElement>('button'),
  ].find((candidate) => candidate.textContent?.includes(text))
  assert.ok(button, `Could not find button containing ${text}`)
  return button
}

async function unmount(rendered: Awaited<ReturnType<typeof renderHistory>>) {
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

describe('AssistantHistory archive controls', () => {
  test('loads active conversations by default and shows archived conversations through the filter', async () => {
    const calls: Array<{ archived: boolean }> = []
    api.get = (async (_url: string, config: unknown) => {
      const archived =
        (config as { params?: { archived?: boolean } } | undefined)?.params
          ?.archived === true
      calls.push({ archived })
      return {
        data: {
          success: true,
          data: {
            conversations: archived
              ? [archivedConversation]
              : [activeConversation, lowerAccessConversation],
          },
        },
      }
    }) as typeof api.get

    const rendered = await renderHistory()
    try {
      assert.match(rendered.container.textContent ?? '', /active-support/)
      assert.match(rendered.container.textContent ?? '', /lower-access-support/)
      assert.doesNotMatch(
        rendered.container.textContent ?? '',
        /archived-support/
      )
      assert.doesNotMatch(
        rendered.container.textContent ?? '',
        /sk-history-secret-123456/
      )
      assert.equal(
        document.querySelectorAll('button[aria-label="Archive conversation"]')
          .length,
        1
      )
      assert.equal(
        document.querySelectorAll('button[aria-label="Restore conversation"]')
          .length,
        0
      )
      assert.equal(calls.length, 1)
      assert.equal(calls[0]?.archived, false)

      await act(async () => {
        findButton('Archived conversations').click()
        await flushEffects()
      })
      await act(flushEffects)

      assert.match(rendered.container.textContent ?? '', /archived-support/)
      assert.doesNotMatch(
        rendered.container.textContent ?? '',
        /active-support/
      )
      assert.equal(calls.at(-1)?.archived, true)
      assert.equal(
        document.querySelectorAll('button[aria-label="Restore conversation"]')
          .length,
        1
      )
    } finally {
      await unmount(rendered)
    }
  })

  test('restores an archived owner conversation and refreshes the current list', async () => {
    let archivedList = true
    let getCalls = 0
    const postCalls: string[] = []
    api.get = (async (_url: string, config: unknown) => {
      getCalls += 1
      const isArchived =
        (config as { params?: { archived?: boolean } } | undefined)?.params
          ?.archived === true
      return {
        data: {
          success: true,
          data: {
            conversations:
              isArchived && archivedList ? [archivedConversation] : [],
          },
        },
      }
    }) as typeof api.get
    api.post = (async (url: string) => {
      postCalls.push(url)
      archivedList = false
      return {
        data: {
          success: true,
          data: {
            id: archivedConversation.id,
            archived: false,
            archived_at: 0,
          },
        },
      }
    }) as typeof api.post

    const rendered = await renderHistory()
    try {
      await act(async () => {
        findButton('Archived conversations').click()
        await flushEffects()
      })
      await act(flushEffects)
      const callsBeforeRestore = getCalls

      await act(async () => {
        findButton('Restore').click()
        await flushEffects()
      })
      await act(flushEffects)

      assert.deepEqual(postCalls, ['/api/assistant/conversations/3/unarchive'])
      assert.ok(getCalls > callsBeforeRestore)
      assert.match(
        rendered.container.textContent ?? '',
        /No archived conversations yet\./
      )
    } finally {
      await unmount(rendered)
    }
  })
})
