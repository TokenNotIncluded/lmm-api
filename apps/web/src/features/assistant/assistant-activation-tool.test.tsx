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
const { AssistantActivationTool } = await import('./assistant-activation-tool')

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

const pendingRequest = {
  id: 9,
  status: 'pending' as const,
  reason: 'Need access for a test integration.',
  source: 'assistant_recommendation' as const,
  ai_recommendation: 'Recommend L1 for a concrete test integration.',
  admin_note: '',
  created_at: 1_786_400_000,
  reviewed_at: 0,
}
const approvedRequest = {
  ...pendingRequest,
  status: 'approved' as const,
  admin_note: 'Approved for testing.',
  reviewed_at: 1_786_500_000,
}
const rejectedRequest = {
  ...pendingRequest,
  status: 'rejected' as const,
  admin_note: 'Please explain which client you will connect.',
  reviewed_at: 1_786_500_000,
}
const recommendationDraft = {
  type: 'l1_recommendation' as const,
  user_statement: 'I need access for a private Claude Code integration.',
  recommendation:
    'Recommend L1 because the user provided a specific client and use case.',
  confirmation_token: 'assistant-confirmation-token',
}

async function flushEffects() {
  await new Promise((resolve) => setTimeout(resolve, 25))
}

async function renderTool(
  options: {
    onContinueSetup?: () => void
    onSubmitted?: () => void
    recommendationDraft?: typeof recommendationDraft
  } = {}
) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const rootRoute = createRootRoute({
    component: () => (
      <QueryClientProvider client={queryClient}>
        <I18nextProvider i18n={i18n}>
          <AssistantActivationTool
            recommendationDraft={options.recommendationDraft}
            onContinueSetup={options.onContinueSetup}
            onSubmitted={options.onSubmitted}
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

async function setTextareaValue(textarea: HTMLTextAreaElement, value: string) {
  const setValue = Object.getOwnPropertyDescriptor(
    HTMLTextAreaElement.prototype,
    'value'
  )?.set
  assert.ok(setValue)
  await act(async () => {
    setValue.call(textarea, value)
    textarea.dispatchEvent(new Event('input', { bubbles: true }))
    await flushEffects()
  })
}

async function waitForCondition(
  condition: () => boolean,
  failureMessage: string
) {
  for (let attempt = 0; attempt < 80; attempt += 1) {
    if (condition()) return
    await flushEffects()
  }
  throw new Error(`${failureMessage}: ${document.body.textContent}`)
}

async function unmount(rendered: Awaited<ReturnType<typeof renderTool>>) {
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

describe('AssistantActivationTool', () => {
  test('shows a direct request fallback when no AI recommendation exists', async () => {
    api.get = (async () => ({
      data: { success: true, data: null },
    })) as typeof api.get

    const rendered = await renderTool()
    try {
      assert.ok(document.querySelector('textarea'))
      assert.equal(document.querySelector('a[href="/wallet"]'), null)
      assert.match(
        document.body.textContent ?? '',
        /without an AI recommendation/
      )
      assert.equal(findButton('Submit for administrator review').disabled, true)
    } finally {
      await unmount(rendered)
    }
  })

  test('submits a direct request without an AI recommendation', async () => {
    api.get = (async () => ({
      data: { success: true, data: null },
    })) as typeof api.get
    let submittedBody: unknown
    api.post = (async (url: string, data: unknown) => {
      assert.equal(url, '/api/user/developer-access/request')
      submittedBody = data
      return { data: { success: true, data: pendingRequest } }
    }) as typeof api.post

    const rendered = await renderTool()
    try {
      const textarea = document.querySelector<HTMLTextAreaElement>('textarea')
      assert.ok(textarea)
      await setTextareaValue(textarea, 'I need L1 for a small test client.')

      await act(async () => {
        findButton('Submit for administrator review').click()
        await flushEffects()
      })
      await waitForCondition(
        () => submittedBody !== undefined,
        'Direct request was not submitted'
      )
      assert.deepEqual(submittedBody, {
        reason: 'I need L1 for a small test client.',
        confirmed: true,
      })
      assert.match(document.body.textContent ?? '', /Recommendation letter/)
    } finally {
      await unmount(rendered)
    }
  })

  test('submits the exact AI recommendation only after confirmation', async () => {
    api.get = (async () => ({
      data: { success: true, data: null },
    })) as typeof api.get
    let submittedBody: unknown
    api.post = (async (url: string, data: unknown) => {
      assert.equal(url, '/api/user/developer-access/request')
      submittedBody = data
      return { data: { success: true, data: pendingRequest } }
    }) as typeof api.post

    let submittedCalls = 0
    const rendered = await renderTool({
      recommendationDraft,
      onSubmitted: () => {
        submittedCalls += 1
      },
    })
    try {
      assert.equal(document.querySelectorAll('textarea').length, 1)
      assert.match(
        document.body.textContent ?? '',
        /Recommend L1 because the user provided a specific client/
      )

      await act(async () => {
        findButton('Confirm and send to administrator').click()
        await flushEffects()
      })
      await waitForCondition(
        () => submittedBody !== undefined,
        'Recommendation was not submitted'
      )
      assert.deepEqual(submittedBody, {
        reason: recommendationDraft.recommendation,
        ai_recommendation: recommendationDraft.recommendation,
        confirmation_token: recommendationDraft.confirmation_token,
        confirmed: true,
      })
      assert.equal(submittedCalls, 1)
      assert.match(document.body.textContent ?? '', /Recommendation letter/)
    } finally {
      await unmount(rendered)
    }
  })

  test('refreshes a pending request and opens setup after approval', async () => {
    let getCalls = 0
    api.get = (async (url: string) => {
      assert.equal(url, '/api/user/developer-access/request')
      getCalls += 1
      return {
        data: {
          success: true,
          data: getCalls === 1 ? pendingRequest : approvedRequest,
        },
      }
    }) as typeof api.get

    let continueCalls = 0
    const rendered = await renderTool({
      onContinueSetup: () => {
        continueCalls += 1
      },
    })
    try {
      await waitForCondition(
        () =>
          document.body.textContent?.includes('Recommendation letter') === true,
        'Pending recommendation state did not render'
      )
      assert.equal(getCalls, 1)

      await act(async () => {
        await rendered.queryClient.refetchQueries({
          queryKey: ['assistant-developer-access-request'],
        })
        await flushEffects()
      })
      await waitForCondition(
        () =>
          document.body.textContent?.includes('L1 access approved') === true,
        'Approved state did not render after refresh'
      )
      assert.equal(getCalls, 2)
      assert.match(document.body.textContent ?? '', /Approved for testing\./)

      await act(async () => {
        findButton('Continue setup').click()
        await flushEffects()
      })
      assert.equal(continueCalls, 1)
    } finally {
      await unmount(rendered)
    }
  })

  test('shows the administrator reply and keeps the direct form after rejection', async () => {
    api.get = (async () => ({
      data: { success: true, data: rejectedRequest },
    })) as typeof api.get

    const rendered = await renderTool()
    try {
      assert.ok(document.querySelector('textarea'))
      assert.match(document.body.textContent ?? '', /Previous request rejected/)
      assert.match(
        document.body.textContent ?? '',
        /Please explain which client you will connect\./
      )
      assert.match(
        document.body.textContent ?? '',
        /Continue the conversation and address the administrator feedback/
      )
    } finally {
      await unmount(rendered)
    }
  })
})
