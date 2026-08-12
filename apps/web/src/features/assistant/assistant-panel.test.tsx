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

import type { AuthUser } from '@/stores/auth-store'

const domWindow = new Window({ url: 'https://console.example.test/' })
for (const key of [
  'window',
  'document',
  'navigator',
  'history',
  'location',
  'HTMLElement',
  'HTMLButtonElement',
  'HTMLFormElement',
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
  'FormData',
  'File',
  'FileReader',
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
const { useAuthStore } = await import('@/stores/auth-store')
const { requestAssistantOpen } = await import('./assistant-events')
const { AssistantLauncher } = await import('./assistant-launcher')
const { AssistantPanel } = await import('./assistant-panel')

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

const assistantStatus = {
  enabled: true,
  model: 'deepseek-v4-flash',
  developer_access_granted: true,
  funding: {
    mode: 'super_administrator' as const,
  },
}

async function flushEffects() {
  await new Promise((resolve) => setTimeout(resolve, 25))
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

async function renderPanel(
  initialPreset?: 'api-key' | 'models' | 'onboarding' | 'plan'
) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const rootRoute = createRootRoute({
    component: () => (
      <QueryClientProvider client={queryClient}>
        <I18nextProvider i18n={i18n}>
          <AssistantPanel
            open
            initialPreset={initialPreset}
            onOpenChange={() => {}}
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

async function renderLauncher(user: AuthUser | null = null) {
  useAuthStore.getState().auth.setUser(user)
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const rootRoute = createRootRoute({
    component: () => (
      <QueryClientProvider client={queryClient}>
        <I18nextProvider i18n={i18n}>
          <AssistantLauncher />
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

afterEach(() => {
  api.get = originalGet
  api.post = originalPost
  useAuthStore.getState().auth.reset('complete')
  window.localStorage.clear()
  window.sessionStorage.clear()
  document.body.replaceChildren()
})

after(() => domWindow.close())

describe('AssistantPanel', () => {
  test('uses an L1 unlock label only for L0 users', async () => {
    api.get = (async (url: string) => {
      if (url === '/api/assistant/status') {
        return { data: { success: true, data: assistantStatus } }
      }
      assert.equal(url, '/api/status')
      return {
        data: {
          success: true,
          data: { assistant: { enabled: true } },
        },
      }
    }) as typeof api.get

    const l0User: AuthUser = {
      id: 7,
      username: 'l0-user',
      role: 1,
      developer_access_granted: false,
    }
    const rendered = await renderLauncher(l0User)
    try {
      const launcherButton = document.querySelector<HTMLButtonElement>(
        '[data-testid="assistant-launcher"]'
      )
      assert.ok(launcherButton)
      assert.equal(launcherButton.textContent?.trim(), 'Unlock L1 with AI')
      assert.equal(
        launcherButton.getAttribute('aria-label'),
        'Unlock L1 with AI'
      )
      assert.equal(launcherButton.getAttribute('title'), 'Unlock L1 with AI')

      await act(async () => {
        useAuthStore.getState().auth.setUser({
          ...l0User,
          developer_access_granted: true,
        })
        await flushEffects()
      })

      assert.equal(launcherButton.textContent?.trim(), 'Service guide')
      assert.equal(
        launcherButton.getAttribute('aria-label'),
        'Open AI assistant'
      )
      assert.equal(launcherButton.getAttribute('title'), 'Open AI assistant')
    } finally {
      await act(async () => rendered.root.unmount())
      rendered.queryClient.clear()
    }
  })

  test('keeps the conversation when the desktop service guide is collapsed and expanded', async () => {
    api.get = (async (url: string) => {
      if (url === '/api/status') {
        return {
          data: {
            success: true,
            data: { assistant: { enabled: true } },
          },
        }
      }
      assert.equal(url, '/api/assistant/status')
      return { data: { success: true, data: assistantStatus } }
    }) as typeof api.get

    const rendered = await renderLauncher()
    try {
      const launcherButton = document.querySelector<HTMLButtonElement>(
        'button[aria-label="Open AI assistant"]'
      )
      assert.ok(launcherButton)
      assert.equal(launcherButton.getAttribute('aria-haspopup'), 'dialog')
      assert.equal(launcherButton.getAttribute('aria-expanded'), 'false')
      assert.equal(
        launcherButton.getAttribute('aria-controls'),
        'ai-assistant-panel'
      )
      await act(async () => {
        launcherButton.click()
        await flushEffects()
      })
      await act(async () =>
        waitForCondition(
          () => document.body.textContent?.includes('How can I help?') === true,
          'Assistant panel did not open'
        )
      )
      assert.equal(launcherButton.getAttribute('aria-expanded'), 'true')
      assert.equal(
        launcherButton.getAttribute('aria-controls'),
        'ai-assistant-panel'
      )
      assert.ok(document.querySelector('#ai-assistant-panel'))
      await act(async () =>
        waitForCondition(
          () =>
            document.body.textContent?.includes(
              'Which option is the best value?'
            ) === true,
          'L1 assistant presets did not render'
        )
      )

      await act(async () => {
        findButton('Which option is the best value?').click()
        await flushEffects()
      })
      assert.match(
        document.body.textContent ?? '',
        /Choose by workload rather than list price\./
      )

      const collapseButton = document.querySelector<HTMLButtonElement>(
        '[data-testid="assistant-collapse"]'
      )
      assert.ok(collapseButton)
      await act(async () => {
        collapseButton.click()
        await flushEffects()
      })
      await act(async () =>
        waitForCondition(
          () =>
            document.querySelector('[data-testid="assistant-expand"]') !== null,
          'Assistant rail did not collapse'
        )
      )

      await act(async () => {
        findButton('Expand').click()
        await flushEffects()
      })
      await act(async () =>
        waitForCondition(
          () =>
            document.body.textContent?.includes(
              'Choose by workload rather than list price.'
            ) === true,
          'Assistant conversation was not restored'
        )
      )
      assert.equal(
        document.querySelector('[data-testid="assistant-collapse"]') !== null,
        true
      )

      const fullscreenButton = document.querySelector<HTMLButtonElement>(
        '[data-testid="assistant-fullscreen"]'
      )
      assert.ok(fullscreenButton)
      await act(async () => {
        fullscreenButton.click()
        await flushEffects()
      })
      assert.ok(document.querySelector('[role="dialog"]'))
      assert.ok(
        document.querySelector('[aria-label="Exit full screen"]')
      )
      await act(async () => {
        document
          .querySelector<HTMLButtonElement>('[aria-label="Exit full screen"]')
          ?.click()
        await flushEffects()
      })
      assert.equal(document.querySelector('[role="dialog"]'), null)
    } finally {
      await act(async () => rendered.root.unmount())
      rendered.queryClient.clear()
    }
  })

  test('syncs queued questions into an already-open assistant input', async () => {
    api.get = (async (url: string) => {
      if (url === '/api/status') {
        return {
          data: {
            success: true,
            data: { assistant: { enabled: true } },
          },
        }
      }
      assert.equal(url, '/api/assistant/status')
      return { data: { success: true, data: assistantStatus } }
    }) as typeof api.get

    const rendered = await renderLauncher()
    try {
      const launcherButton = document.querySelector<HTMLButtonElement>(
        'button[aria-label="Open AI assistant"]'
      )
      assert.ok(launcherButton)
      await act(async () => {
        launcherButton.click()
        await flushEffects()
      })
      await act(async () =>
        waitForCondition(
          () => document.querySelector('textarea') !== null,
          'Assistant input did not render'
        )
      )

      const textarea = document.querySelector<HTMLTextAreaElement>('textarea')
      assert.ok(textarea)
      const question = 'How do I set up Claude Code or CC Switch?'

      await act(async () => {
        requestAssistantOpen(undefined, question)
        await flushEffects()
      })
      assert.equal(textarea.value, question)

      await setTextareaValue(textarea, 'draft that should be replaced')
      await act(async () => {
        requestAssistantOpen(undefined, question)
        await flushEffects()
      })
      assert.equal(textarea.value, question)
    } finally {
      await act(async () => rendered.root.unmount())
      rendered.queryClient.clear()
    }
  })

  test('appends guided presets without replacing the current conversation', async () => {
    api.get = (async (url: string) => {
      assert.equal(url, '/api/assistant/status')
      return { data: { success: true, data: assistantStatus } }
    }) as typeof api.get

    const rendered = await renderPanel('api-key')
    try {
      assert.match(
        document.body.textContent ?? '',
        /What are my Base URL, model ID, and API key\?/
      )

      await act(async () => {
        requestAssistantOpen('plan')
        await flushEffects()
      })

      assert.match(
        document.body.textContent ?? '',
        /What are my Base URL, model ID, and API key\?/
      )
      assert.match(
        document.body.textContent ?? '',
        /Which option is the best value\?/
      )
    } finally {
      await act(async () => rendered.root.unmount())
      rendered.queryClient.clear()
    }
  })

  test('keeps L0 guidance useful without exposing account or payment actions', async () => {
    api.get = (async (url: string) => {
      if (url === '/api/assistant/status') {
        return {
          data: {
            success: true,
            data: { ...assistantStatus, developer_access_granted: false },
          },
        }
      }
      if (url === '/api/user/developer-access/request') {
        return { data: { success: true, data: null } }
      }
      throw new Error(`Unexpected GET ${url}`)
    }) as typeof api.get

    const rendered = await renderPanel('onboarding')
    try {
      await act(async () =>
        waitForCondition(
          () => document.querySelector('textarea') !== null,
          'L0 access request did not render'
        )
      )
      assert.match(
        document.body.textContent ?? '',
        /Ask an administrator to raise my access level/
      )
      assert.doesNotMatch(
        document.body.textContent ?? '',
        /What are my Base URL, model ID, and API key\?/
      )
      assert.equal(document.querySelector('a[href="/wallet"]'), null)
      assert.doesNotMatch(
        document.body.textContent ?? '',
        /Your wallet is charged/
      )

      await act(async () => {
        findButton('Clear conversation').click()
        await flushEffects()
      })
      assert.ok(findButton('Ask an administrator to raise my access level'))
      assert.throws(() => findButton('Which option is the best value?'))
      assert.throws(() => findButton('How is request cost calculated?'))
      assert.throws(() =>
        findButton('What can I do while access is under review?')
      )
      assert.throws(() =>
        findButton('How do I set up Claude Code or CC Switch?')
      )

      assert.doesNotMatch(document.body.textContent ?? '', /save 20%/)
      assert.equal(document.querySelector('#assistant-expected-credit'), null)
      assert.equal(document.querySelector('#assistant-topup-credit'), null)
      assert.equal(document.querySelector('a[href="/wallet"]'), null)

      assert.throws(() => findButton('Create API key'))
      assert.equal(document.querySelector('a[href="/wallet"]'), null)
    } finally {
      await act(async () => rendered.root.unmount())
      rendered.queryClient.clear()
    }
  })

  test('does not mistake a failed access check for L0 or expose presets', async () => {
    api.get = (async (url: string) => {
      assert.equal(url, '/api/assistant/status')
      throw new Error('status unavailable')
    }) as typeof api.get

    const rendered = await renderPanel()
    try {
      await act(async () =>
        waitForCondition(
          () =>
            document.body.textContent?.includes(
              'Unable to verify account access'
            ) === true,
          'Assistant did not render the access error'
        )
      )
      assert.doesNotMatch(
        document.body.textContent ?? '',
        /Ask an administrator to raise my access level/
      )
      assert.doesNotMatch(
        document.body.textContent ?? '',
        /Which option is the best value\?/
      )
      assert.ok(findButton('Retry'))
    } finally {
      await act(async () => rendered.root.unmount())
      rendered.queryClient.clear()
    }
  })

  test('shows the signed-in model IDs inside the assistant', async () => {
    api.get = (async (url: string) => {
      if (url === '/api/assistant/status') {
        return { data: { success: true, data: assistantStatus } }
      }
      if (url === '/api/user/models') {
        return {
          data: {
            success: true,
            data: ['claude-3-7-sonnet', 'deepseek-v4-flash'],
          },
        }
      }
      throw new Error(`Unexpected GET ${url}`)
    }) as typeof api.get

    const rendered = await renderPanel('models')
    try {
      await act(async () => {
        findButton('View all currently available models').click()
        await flushEffects()
      })
      await act(async () =>
        waitForCondition(
          () =>
            document.body.textContent?.includes('claude-3-7-sonnet') === true,
          'Assistant model IDs did not render'
        )
      )
      assert.match(document.body.textContent ?? '', /deepseek-v4-flash/)
      assert.doesNotMatch(
        document.body.textContent ?? '',
        /Default assistant model/
      )
      assert.ok(document.querySelector('button[aria-label="Copy model names"]'))
    } finally {
      await act(async () => rendered.root.unmount())
      rendered.queryClient.clear()
    }
  })

  test('shows that the super administrator funds assistant usage', async () => {
    api.get = (async (url: string) => {
      assert.equal(url, '/api/assistant/status')
      return { data: { success: true, data: assistantStatus } }
    }) as typeof api.get

    const rendered = await renderPanel()
    try {
      assert.match(
        document.body.textContent ?? '',
        /Funded by the super administrator/
      )
      assert.match(
        document.body.textContent ?? '',
        /charged to the super administrator account, not your wallet/
      )
      assert.doesNotMatch(document.body.textContent ?? '', /Weekly included/)
    } finally {
      await act(async () => rendered.root.unmount())
      rendered.queryClient.clear()
    }
  })

  test('gives administrators a confirmation-gated server change card', async () => {
    let appliedRequest: unknown
    api.get = (async (url: string) => {
      assert.equal(url, '/api/assistant/status')
      return {
        data: {
          success: true,
          data: {
            ...assistantStatus,
            role: 10,
            is_admin: true,
            access_level: 'ADMIN',
            capabilities: {
              account: true,
              admin_config: true,
              admin_pricing: true,
            },
          },
        },
      }
    }) as typeof api.get
    api.post = (async (url: string, data: unknown) => {
      if (url === '/api/assistant/chat') {
        return {
          data: {
            choices: [{ message: { content: 'I prepared the exact preview.' } }],
            lmm_assistant_action: {
              type: 'admin_config_change',
              confirmation_token: 'admin-secret-token',
              requires_confirmation: true,
              expires_in_seconds: 600,
              changes: [
                {
                  key: 'DefaultCollapseSidebar',
                  label: 'Collapse the main sidebar by default',
                  old_value: 'false',
                  new_value: 'true',
                },
              ],
            },
          },
          headers: {},
        }
      }
      assert.equal(url, '/api/assistant/admin/apply')
      appliedRequest = data
      return {
        data: {
          success: true,
          data: { applied: true, kind: 'config' },
        },
      }
    }) as typeof api.post

    const rendered = await renderPanel()
    try {
      assert.match(document.body.textContent ?? '', /ADMIN · Administrator mode/)
      const textarea = document.querySelector<HTMLTextAreaElement>(
        'textarea[placeholder="Ask about server configuration, model pricing, or operations..."]'
      )
      assert.ok(textarea)
      await setTextareaValue(textarea, 'Turn on the desktop sidebar default.')
      await act(async () => {
        document.querySelector<HTMLButtonElement>('button[aria-label="Submit"]')?.click()
        await flushEffects()
      })
      await act(async () =>
        waitForCondition(
          () =>
            document.body.textContent?.includes(
              'Administrator configuration change'
            ) === true,
          'Administrator preview did not render'
        )
      )
      assert.doesNotMatch(document.body.textContent ?? '', /admin-secret-token/)
      await act(async () => {
        findButton('Confirm and apply').click()
        await flushEffects()
      })
      await act(async () =>
        waitForCondition(
          () =>
            document.body.textContent?.includes('Administrator change applied') ===
            true,
          'Administrator change result did not render'
        )
      )
      assert.deepEqual(appliedRequest, {
        confirmation_token: 'admin-secret-token',
        confirmed: true,
      })
    } finally {
      await act(async () => rendered.root.unmount())
      rendered.queryClient.clear()
    }
  })

  test('shows permitted history while redacting sensitive content and keeping private cards owner-only', async () => {
    api.get = (async (url: string) => {
      if (url === '/api/assistant/status') {
        return { data: { success: true, data: assistantStatus } }
      }
      if (url === '/api/assistant/conversations') {
        return {
          data: {
            success: true,
            data: {
              privacy_notice: 'Conversations are not private.',
              conversations: [
                {
                  id: 1,
                  title: 'CC Switch setup',
                  last_message_preview: 'How do I configure CC Switch?',
                  owner: 'self',
                  created_at: 1_786_400_000,
                  updated_at: 1_786_400_001,
                  privacy_notice: 'Conversations are not private.',
                },
                {
                  id: 2,
                  title: 'Credential support',
                  last_message_preview:
                    'email is private@example.test and API key: sk-history-secret-123456',
                  owner: 'lower_level_user',
                  created_at: 1_786_400_000,
                  updated_at: 1_786_400_002,
                  privacy_notice: 'Conversations are not private.',
                },
              ],
            },
          },
        }
      }
      if (url === '/api/assistant/conversations/2') {
        return {
          data: {
            success: true,
            data: {
              conversation: {
                id: 2,
                title: 'Credential support',
                last_message_preview:
                  'email is private@example.test and API key: sk-history-secret-123456',
                owner: 'lower_level_user',
                created_at: 1_786_400_000,
                updated_at: 1_786_400_002,
                privacy_notice: 'Conversations are not private.',
              },
              messages: [
                {
                  id: 2,
                  role: 'user',
                  content:
                    'email is private@example.test and API key: sk-history-secret-123456',
                  created_at: 1_786_400_002,
                  cards: [
                    {
                      type: 'protected',
                      label: 'Private credential',
                      owner: 'protected',
                      shield: true,
                    },
                  ],
                },
              ],
              privacy_notice: 'Conversations are not private.',
            },
          },
        }
      }
      throw new Error(`Unexpected GET ${url}`)
    }) as typeof api.get

    const rendered = await renderPanel()
    try {
      assert.match(
        document.body.textContent ?? '',
        /Your assistant conversations are not private/
      )
      await act(async () => {
        findButton('Conversation history').click()
        await flushEffects()
      })
      await act(async () =>
        waitForCondition(
          () =>
            document.body.textContent?.includes('Lower-access user conversation') ===
            true,
          'Assistant history did not render'
        )
      )
      assert.doesNotMatch(document.body.textContent ?? '', /private@example\.test/)
      assert.doesNotMatch(document.body.textContent ?? '', /sk-history-secret-123456/)

      await act(async () => {
        const viewButtons = [
          ...document.querySelectorAll<HTMLButtonElement>('button'),
        ].filter((button) => button.textContent?.trim() === 'View')
        assert.equal(viewButtons.length, 2)
        viewButtons[1]?.click()
        await flushEffects()
      })
      await act(async () =>
        waitForCondition(
          () =>
            document.body.textContent?.includes(
              'Private cards remain visible only to their owner'
            ) === true,
          'Assistant history detail did not render'
        )
      )
      assert.equal(
        document.querySelector('[data-testid="assistant-private-card-value"]'),
        null
      )
    } finally {
      await act(async () => rendered.root.unmount())
      rendered.queryClient.clear()
    }
  })

  test('handles forbidden conversation history without exposing an HTTP error', async () => {
    api.get = (async (url: string) => {
      if (url === '/api/assistant/status') {
        return { data: { success: true, data: assistantStatus } }
      }
      assert.equal(url, '/api/assistant/conversations')
      throw { response: { status: 403 } }
    }) as typeof api.get

    const rendered = await renderPanel()
    try {
      await act(async () => {
        findButton('Conversation history').click()
        await flushEffects()
      })
      await act(async () =>
        waitForCondition(
          () =>
            document.body.textContent?.includes(
              'Conversation history is not available to this account.'
            ) === true,
          'Forbidden history message did not render'
        )
      )
      assert.doesNotMatch(document.body.textContent ?? '', /HTTP 403/)
    } finally {
      await act(async () => rendered.root.unmount())
      rendered.queryClient.clear()
    }
  })

  test('handles a missing conversation history endpoint without exposing an HTTP error', async () => {
    api.get = (async (url: string) => {
      if (url === '/api/assistant/status') {
        return { data: { success: true, data: assistantStatus } }
      }
      assert.equal(url, '/api/assistant/conversations')
      throw { response: { status: 404 } }
    }) as typeof api.get

    const rendered = await renderPanel()
    try {
      await act(async () => {
        findButton('Conversation history').click()
        await flushEffects()
      })
      await act(async () =>
        waitForCondition(
          () =>
            document.body.textContent?.includes(
              'This conversation no longer exists or is unavailable.'
            ) === true,
          'Missing history message did not render'
        )
      )
      assert.doesNotMatch(document.body.textContent ?? '', /HTTP 404/)
    } finally {
      await act(async () => rendered.root.unmount())
      rendered.queryClient.clear()
    }
  })

  test('does not place sensitive input into the chat transcript or send it to the assistant', async () => {
    let posted = 0
    api.get = (async (url: string) => {
      assert.equal(url, '/api/assistant/status')
      return { data: { success: true, data: assistantStatus } }
    }) as typeof api.get
    api.post = (async () => {
      posted += 1
      throw new Error('Sensitive input must not be sent')
    }) as typeof api.post

    const rendered = await renderPanel()
    try {
      const textarea = document.querySelector<HTMLTextAreaElement>('textarea')
      assert.ok(textarea)
      await setTextareaValue(textarea, 'my email is private@example.test')
      const submit = document.querySelector<HTMLButtonElement>(
        'button[aria-label="Submit"]'
      )
      assert.ok(submit)
      await act(async () => {
        submit.click()
        await flushEffects()
      })
      assert.equal(posted, 0)
      assert.match(
        document.body.textContent ?? '',
        /Sensitive message was not sent/
      )
      assert.doesNotMatch(document.body.textContent ?? '', /private@example\.test/)
    } finally {
      await act(async () => rendered.root.unmount())
      rendered.queryClient.clear()
    }
  })

  test('opens an explicit confirmation for an AI L1 recommendation', async () => {
    let submittedRecommendation: unknown
    api.get = (async (url: string) => {
      if (url === '/api/assistant/status') {
        return {
          data: {
            success: true,
            data: { ...assistantStatus, developer_access_granted: false },
          },
        }
      }
      if (url === '/api/user/developer-access/request') {
        return { data: { success: true, data: null } }
      }
      throw new Error(`Unexpected GET ${url}`)
    }) as typeof api.get
    api.post = (async (url: string, data: unknown) => {
      if (url === '/api/assistant/chat') {
        return {
          data: {
            choices: [
              {
                message: {
                  content: 'I have enough detail to recommend L1 access.',
                },
              },
            ],
            lmm_assistant_action: {
              type: 'l1_recommendation',
              user_statement: 'I will connect Claude Code for private work.',
              recommendation:
                'Recommend L1 because the user identified a specific client and purpose.',
              confirmation_token: 'assistant-confirmation-token',
            },
          },
          headers: { 'x-lmm-assistant-intent': 'onboarding' },
        }
      }
      assert.equal(url, '/api/user/developer-access/request')
      submittedRecommendation = data
      return {
        data: {
          success: true,
          data: {
            id: 11,
            status: 'pending',
            reason: 'I will connect Claude Code for private work.',
            source: 'assistant_recommendation',
            ai_recommendation:
              'Recommend L1 because the user identified a specific client and purpose.',
            admin_note: '',
            created_at: 1_786_400_000,
            reviewed_at: 0,
          },
        },
      }
    }) as typeof api.post

    const rendered = await renderPanel()
    try {
      const textarea = document.querySelector<HTMLTextAreaElement>(
        'textarea[placeholder="Write a short explanation of what you want to build or why you need L1 access."]'
      )
      assert.ok(textarea)
      await setTextareaValue(
        textarea,
        'I want Claude Code access for my private project.'
      )
      const submit = document.querySelector<HTMLButtonElement>(
        'button[aria-label="Submit"]'
      )
      assert.ok(submit)
      await act(async () => {
        submit.click()
        await flushEffects()
      })
      await act(async () =>
        waitForCondition(
          () =>
            document.body.textContent?.includes('Confirm AI recommendation') ===
            true,
          'AI recommendation confirmation did not render'
        )
      )
      assert.match(
        document.body.textContent ?? '',
        /I will connect Claude Code for private work\./
      )

      await act(async () => {
        findButton('Confirm and send to administrator').click()
        await flushEffects()
      })
      await act(async () =>
        waitForCondition(
          () => submittedRecommendation !== undefined,
          'Confirmed recommendation was not submitted'
        )
      )
      assert.deepEqual(submittedRecommendation, {
        reason: 'I will connect Claude Code for private work.',
        ai_recommendation:
          'Recommend L1 because the user identified a specific client and purpose.',
        confirmation_token: 'assistant-confirmation-token',
        confirmed: true,
      })
    } finally {
      await act(async () => rendered.root.unmount())
      rendered.queryClient.clear()
    }
  })

  test('retries the exact failed conversation without duplicating the user message', async () => {
    const posted: unknown[] = []
    api.get = (async (url: string) => {
      assert.equal(url, '/api/assistant/status')
      return { data: { success: true, data: assistantStatus } }
    }) as typeof api.get
    api.post = (async (url: string, data: unknown) => {
      assert.equal(url, '/api/assistant/chat')
      posted.push(data)
      if (posted.length === 1) throw new Error('assistant offline')
      return {
        data: {
          choices: [
            { message: { content: 'Use the verified Windows guide.' } },
          ],
        },
        headers: { 'x-lmm-assistant-intent': 'client_setup' },
      }
    }) as typeof api.post

    const rendered = await renderPanel()
    const textarea = document.querySelector<HTMLTextAreaElement>(
      'textarea[placeholder="Ask about plans, setup, keys, or costs..."]'
    )
    assert.ok(textarea)
    assert.match(
      document.body.textContent ?? '',
      /AI customer-service token usage is charged to the super administrator account, not your wallet\./
    )
    await setTextareaValue(textarea, 'How do I configure Claude Code?')
    const submit = document.querySelector<HTMLButtonElement>(
      'button[aria-label="Submit"]'
    )
    assert.ok(submit)

    await act(async () => {
      submit.click()
      await flushEffects()
    })
    await act(async () =>
      waitForCondition(
        () => document.body.textContent?.includes('Contact support') === true,
        'Assistant error actions did not render'
      )
    )
    assert.match(
      document.body.textContent ?? '',
      /The AI assistant could not answer right now/
    )
    assert.equal(posted.length, 1)

    await act(async () => {
      findButton('Retry').click()
      await flushEffects()
    })
    await act(async () =>
      waitForCondition(
        () =>
          document.body.textContent?.includes(
            'Use the verified Windows guide.'
          ) === true,
        'Retried assistant answer did not render'
      )
    )

    assert.equal(posted.length, 2)
    assert.deepEqual(posted[1], posted[0])
    assert.doesNotMatch(
      document.body.textContent ?? '',
      /The AI assistant could not answer right now/
    )
    assert.equal(
      (document.body.textContent ?? '').match(
        /How do I configure Claude Code\?/g
      )?.length,
      1
    )

    await act(async () => rendered.root.unmount())
    rendered.queryClient.clear()
  })

  test('renders assistant Markdown as safe structured content', async () => {
    api.get = (async (url: string) => {
      assert.equal(url, '/api/assistant/status')
      return { data: { success: true, data: assistantStatus } }
    }) as typeof api.get
    api.post = (async (url: string) => {
      assert.equal(url, '/api/assistant/chat')
      return {
        data: {
          choices: [
            {
              message: {
                content:
                  '## Claude Code\n\n1. Install the client.\n2. Set the Base URL to `/v1`.\n\n**Keep your key private.**',
              },
            },
          ],
        },
        headers: { 'x-lmm-assistant-intent': 'client_setup' },
      }
    }) as typeof api.post

    const rendered = await renderPanel()
    try {
      const textarea = document.querySelector<HTMLTextAreaElement>(
        'textarea[placeholder="Ask about plans, setup, keys, or costs..."]'
      )
      assert.ok(textarea)
      await setTextareaValue(textarea, 'How do I configure Claude Code?')
      const submit = document.querySelector<HTMLButtonElement>(
        'button[aria-label="Submit"]'
      )
      assert.ok(submit)

      await act(async () => {
        submit.click()
        await flushEffects()
      })
      await act(async () =>
        waitForCondition(
          () => document.body.textContent?.includes('Keep your key private.') === true,
          'Markdown assistant answer did not render'
        )
      )

      assert.ok(document.querySelector('h2'))
      assert.equal(document.querySelectorAll('ol > li').length, 2)
      assert.ok(document.querySelector('code'))
      assert.ok(document.querySelector('strong'))
      assert.equal(document.querySelector('script'), null)
    } finally {
      await act(async () => rendered.root.unmount())
      rendered.queryClient.clear()
    }
  })
})
