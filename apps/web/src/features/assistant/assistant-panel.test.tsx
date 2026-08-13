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

const ASSISTANT_PRIVACY_NOTICE_COLLAPSE_DELAY_MS = 5_000

const originalGet = api.get
const originalPost = api.post
const originalMatchMedia = window.matchMedia
const originalInnerWidth = window.innerWidth
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
  initialPreset?: 'api-key' | 'models' | 'onboarding' | 'plan',
  mode: 'mobile' | 'rail' = 'mobile',
  user: AuthUser | null = null,
  handoff?: {
    initialMessage: string
    autoSendRequestId: string
    onAutoSendConsumed?: (requestId: string) => void
  }
) {
  useAuthStore.getState().auth.setUser(user)
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const rootRoute = createRootRoute({
    component: () => (
      <QueryClientProvider client={queryClient}>
        <I18nextProvider i18n={i18n}>
          <AssistantPanel
            open
            mode={mode}
            initialPreset={initialPreset}
            initialMessage={handoff?.initialMessage}
            autoSendRequestId={handoff?.autoSendRequestId}
            onAutoSendConsumed={handoff?.onAutoSendConsumed}
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
  Object.defineProperty(window, 'innerWidth', {
    configurable: true,
    value: 1280,
  })
  window.matchMedia = ((query: string) => ({
    matches: query === '(min-width: 1280px)',
    media: query,
    onchange: null,
    addEventListener: () => {},
    removeEventListener: () => {},
    addListener: () => {},
    removeListener: () => {},
    dispatchEvent: () => false,
  })) as typeof window.matchMedia
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

function findCard(text: string): HTMLElement | null {
  return (
    [...document.querySelectorAll<HTMLElement>('[data-slot="card"]')].find(
      (card) => card.textContent?.includes(text)
    ) ?? null
  )
}

const originalSetTimeout = globalThis.setTimeout
const originalClearTimeout = globalThis.clearTimeout
const privacyNoticeTimerHandle = {} as ReturnType<typeof setTimeout>
let capturedPrivacyNoticeTimer: (() => void) | null = null

function capturePrivacyNoticeTimer() {
  capturedPrivacyNoticeTimer = null
  globalThis.setTimeout = ((callback: () => void, delay?: number) => {
    if (
      typeof callback === 'function' &&
      delay === ASSISTANT_PRIVACY_NOTICE_COLLAPSE_DELAY_MS
    ) {
      capturedPrivacyNoticeTimer = () => callback()
      return privacyNoticeTimerHandle
    }
    return originalSetTimeout(callback, delay)
  }) as typeof globalThis.setTimeout
  globalThis.clearTimeout = ((handle) => {
    if (handle === privacyNoticeTimerHandle) {
      capturedPrivacyNoticeTimer = null
      return
    }
    return originalClearTimeout(handle)
  }) as typeof globalThis.clearTimeout

  return () => {
    globalThis.setTimeout = originalSetTimeout
    globalThis.clearTimeout = originalClearTimeout
    capturedPrivacyNoticeTimer = null
  }
}

function fireCapturedPrivacyNoticeTimer() {
  const callback = capturedPrivacyNoticeTimer
  if (!callback) throw new Error('Privacy notice collapse timer was not set')
  capturedPrivacyNoticeTimer = null
  callback()
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
  globalThis.setTimeout = originalSetTimeout
  globalThis.clearTimeout = originalClearTimeout
  capturedPrivacyNoticeTimer = null
  api.get = originalGet
  api.post = originalPost
  window.matchMedia = originalMatchMedia
  Object.defineProperty(window, 'innerWidth', {
    configurable: true,
    value: originalInnerWidth,
  })
  useAuthStore.getState().auth.reset('complete')
  window.localStorage.clear()
  window.sessionStorage.clear()
  document.body.replaceChildren()
})

after(() => domWindow.close())

describe('AssistantPanel', () => {
  test('auto-collapses the privacy notice without moving focus and can reopen it', async () => {
    api.get = (async (url: string) => {
      assert.equal(url, '/api/assistant/status')
      return { data: { success: true, data: assistantStatus } }
    }) as typeof api.get

    const restoreTimers = capturePrivacyNoticeTimer()
    const rendered = await renderPanel()
    try {
      const toggle = document.querySelector<HTMLButtonElement>(
        '[data-testid="assistant-privacy-notice-toggle"]'
      )
      assert.ok(toggle)
      assert.equal(toggle.getAttribute('aria-expanded'), 'true')
      const privacyDescription = document.querySelector(
        '#assistant-privacy-notice-description'
      )?.textContent
      assert.match(
        privacyDescription ?? '',
        /Your assistant conversations are not private\. Authorized higher-access users may review them\./
      )
      assert.match(
        privacyDescription ?? '',
        /Do not send personal information, passwords, API keys, or credentials in chat\./
      )
      assert.match(
        privacyDescription ?? '',
        /Pattern matching is not a guarantee\./
      )

      toggle.focus()
      await act(async () => {
        fireCapturedPrivacyNoticeTimer()
        await flushEffects()
      })
      assert.equal(toggle.getAttribute('aria-expanded'), 'false')
      assert.equal(document.activeElement, toggle)
      assert.match(
        document.querySelector('#assistant-privacy-notice-description')
          ?.className ?? '',
        /sr-only/
      )

      await act(async () => {
        toggle.click()
        await flushEffects()
      })
      assert.equal(toggle.getAttribute('aria-expanded'), 'true')
      assert.doesNotMatch(
        document.querySelector('#assistant-privacy-notice-description')
          ?.className ?? '',
        /sr-only/
      )

      await act(async () => {
        fireCapturedPrivacyNoticeTimer()
        await flushEffects()
      })
      assert.equal(toggle.getAttribute('aria-expanded'), 'false')
    } finally {
      await act(async () => rendered.root.unmount())
      rendered.queryClient.clear()
      restoreTimers()
    }
  })

  test('renders the mobile assistant sheet at the full dynamic viewport size', async () => {
    api.get = (async (url: string) => {
      assert.equal(url, '/api/assistant/status')
      return { data: { success: true, data: assistantStatus } }
    }) as typeof api.get

    const rendered = await renderPanel(undefined, 'mobile')
    try {
      const sheetContent = document.querySelector<HTMLElement>(
        '[data-slot="sheet-content"]'
      )
      assert.ok(sheetContent)
      assert.match(sheetContent.className, /h-dvh/)
      assert.match(sheetContent.className, /w-screen/)
      assert.match(sheetContent.className, /max-w-none/)
      assert.match(sheetContent.className, /rounded-none/)
      assert.ok(sheetContent.querySelector('[data-slot="sheet-close"]'))
      assert.ok(sheetContent.querySelector('textarea'))
      assert.match(
        sheetContent.textContent ?? '',
        /Your assistant conversations are not private/
      )
    } finally {
      await act(async () => rendered.root.unmount())
      rendered.queryClient.clear()
    }
  })

  test('auto-sends a homepage handoff exactly once after access is confirmed', async () => {
    let posted = 0
    const consumedIds: string[] = []
    api.get = (async (url: string) => {
      assert.equal(url, '/api/assistant/status')
      return { data: { success: true, data: assistantStatus } }
    }) as typeof api.get
    api.post = (async (url: string, data: unknown) => {
      assert.equal(url, '/api/assistant/chat')
      posted += 1
      assert.match(JSON.stringify(data), /Help me configure the SDK/)
      return {
        data: {
          choices: [{ message: { content: 'SDK guidance is ready.' } }],
        },
        headers: {},
      }
    }) as typeof api.post

    const rendered = await renderPanel(undefined, 'mobile', null, {
      initialMessage: 'Help me configure the SDK',
      autoSendRequestId: 'home-handoff-1',
      onAutoSendConsumed: (requestId) => consumedIds.push(requestId),
    })
    try {
      await act(async () =>
        waitForCondition(
          () =>
            document.body.textContent?.includes('SDK guidance is ready.') ===
            true,
          'Homepage handoff did not send'
        )
      )
      await act(flushEffects)
      assert.equal(posted, 1)
      assert.deepEqual(consumedIds, ['home-handoff-1'])
      assert.equal(
        [...document.querySelectorAll('.is-user')].filter((message) =>
          message.textContent?.includes('Help me configure the SDK')
        ).length,
        1
      )
    } finally {
      await act(async () => rendered.root.unmount())
      rendered.queryClient.clear()
    }
  })

  test('hides an active tool card for a new ordinary message and allows reopening it', async () => {
    api.get = (async (url: string) => {
      if (url === '/api/assistant/status') {
        return { data: { success: true, data: assistantStatus } }
      }
      assert.equal(url, '/api/user/models')
      return {
        data: {
          success: true,
          data: ['claude-3-7-sonnet', 'deepseek-v4-flash'],
        },
      }
    }) as typeof api.get
    api.post = (async (url: string) => {
      assert.equal(url, '/api/assistant/chat')
      return {
        data: {
          choices: [{ message: { content: 'A fresh ordinary answer.' } }],
        },
        headers: {},
      }
    }) as typeof api.post

    const rendered = await renderPanel('models')
    try {
      await act(async () =>
        waitForCondition(
          () => findCard('View all currently available models') !== null,
          'Model tool card did not render'
        )
      )

      const textarea = document.querySelector<HTMLTextAreaElement>('textarea')
      assert.ok(textarea)
      await setTextareaValue(textarea, 'Start a fresh ordinary message.')
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
            document.body.textContent?.includes('A fresh ordinary answer.') ===
            true,
          'Fresh ordinary answer did not render'
        )
      )
      assert.equal(findCard('View all currently available models'), null)

      await act(async () => {
        requestAssistantOpen('models')
        await flushEffects()
      })
      await act(async () =>
        waitForCondition(
          () => findCard('View all currently available models') !== null,
          'Model tool card did not reopen'
        )
      )
    } finally {
      await act(async () => rendered.root.unmount())
      rendered.queryClient.clear()
    }
  })

  test('removes response action cards when a new question starts', async () => {
    let requestCount = 0
    api.get = (async (url: string) => {
      assert.equal(url, '/api/assistant/status')
      return { data: { success: true, data: assistantStatus } }
    }) as typeof api.get
    api.post = (async (url: string) => {
      assert.equal(url, '/api/assistant/chat')
      requestCount += 1
      return {
        data: {
          choices: [
            {
              message: {
                content:
                  requestCount === 1
                    ? 'Here are the model IDs.'
                    : 'The new question has a clean response.',
              },
            },
          ],
        },
        headers:
          requestCount === 1 ? { 'x-lmm-assistant-intent': 'models' } : {},
      }
    }) as typeof api.post

    const rendered = await renderPanel()
    try {
      const textarea = document.querySelector<HTMLTextAreaElement>('textarea')
      assert.ok(textarea)

      await setTextareaValue(textarea, 'Which models can I use?')
      await act(async () => {
        findButton('Send').click()
        await flushEffects()
      })
      await act(async () =>
        waitForCondition(
          () =>
            document.body.textContent?.includes(
              'View all currently available models'
            ) === true,
          'Response action card did not render'
        )
      )

      await setTextareaValue(textarea, 'How do I continue?')
      await act(async () => {
        findButton('Send').click()
        await flushEffects()
      })
      await act(async () =>
        waitForCondition(
          () =>
            document.body.textContent?.includes(
              'The new question has a clean response.'
            ) === true,
          'Second assistant response did not render'
        )
      )

      assert.doesNotMatch(
        document.body.textContent ?? '',
        /View all currently available models/
      )
      assert.match(document.body.textContent ?? '', /Here are the model IDs\./)
    } finally {
      await act(async () => rendered.root.unmount())
      rendered.queryClient.clear()
    }
  })

  test('clears tool state and local entries when clearing the conversation', async () => {
    api.get = (async (url: string) => {
      if (url === '/api/assistant/status') {
        return { data: { success: true, data: assistantStatus } }
      }
      assert.equal(url, '/api/user/models')
      return { data: { success: true, data: ['claude-3-7-sonnet'] } }
    }) as typeof api.get

    const rendered = await renderPanel('models')
    try {
      await act(async () =>
        waitForCondition(
          () => findCard('View all currently available models') !== null,
          'Model tool card did not render'
        )
      )

      await act(async () => {
        findButton('Clear conversation').click()
        await flushEffects()
      })
      assert.equal(findCard('View all currently available models'), null)
      assert.match(document.body.textContent ?? '', /How can I help\?/)
      assert.throws(() => findButton('Clear conversation'))
    } finally {
      await act(async () => rendered.root.unmount())
      rendered.queryClient.clear()
    }
  })

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
      await act(async () => {
        requestAssistantOpen('plan')
        await flushEffects()
      })
      await act(async () =>
        waitForCondition(
          () =>
            document.body.textContent?.includes(
              'Live plan and discount advisor'
            ) === true,
          'Plan tool did not open from the shortcut'
        )
      )
      assert.doesNotMatch(
        document.body.textContent ?? '',
        /Which option is the best value\?/
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
              'Live plan and discount advisor'
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
      assert.ok(document.querySelector('[aria-label="Exit full screen"]'))
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

  test('opens a tool shortcut without appending a canned question', async () => {
    api.get = (async (url: string) => {
      assert.equal(url, '/api/assistant/status')
      return { data: { success: true, data: assistantStatus } }
    }) as typeof api.get

    const rendered = await renderPanel('api-key')
    try {
      await act(async () =>
        waitForCondition(
          () =>
            document.body.textContent?.includes('Create a default API key') ===
            true,
          'API key tool did not open'
        )
      )
      assert.doesNotMatch(
        document.body.textContent ?? '',
        /What are my Base URL, model ID, and API key\?/
      )

      await act(async () => {
        requestAssistantOpen('plan')
        await flushEffects()
      })

      await act(async () =>
        waitForCondition(
          () =>
            document.body.textContent?.includes(
              'Live plan and discount advisor'
            ) === true,
          'Plan tool did not open'
        )
      )
      assert.doesNotMatch(
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
      assert.match(document.body.textContent ?? '', /Unlock L1 with AI/)
      assert.doesNotMatch(
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
      assert.doesNotMatch(
        document.body.textContent ?? '',
        /Ask an administrator to raise my access level/
      )
      assert.throws(() => findButton('Which option is the best value?'))
      assert.throws(() => findButton('How is request cost calculated?'))
      assert.ok(findButton('What can I do while access is under review?'))
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

  test('shows the onboarding todo only after developer access is granted', async () => {
    api.get = (async (url: string) => {
      if (url === '/api/assistant/status') {
        return { data: { success: true, data: assistantStatus } }
      }
      assert.equal(url, '/api/user/self/onboarding/todo')
      return {
        data: {
          success: true,
          data: {
            eligibility: {
              eligible: true,
              developer_access_granted: true,
              trust_level: 1,
            },
            status: 'in_progress',
            current_step: 'create_api_key',
            steps: [
              { id: 'create_api_key', status: 'pending' },
              { id: 'install_client', status: 'pending' },
              { id: 'configure_client', status: 'pending' },
              { id: 'first_successful_response', status: 'pending' },
            ],
          },
        },
      }
    }) as typeof api.get

    const rendered = await renderPanel(undefined, 'mobile', {
      id: 42,
      username: 'l1-user',
      role: 1,
      developer_access_granted: true,
      onboarding: {
        activation_complete: true,
        credential_complete: false,
        first_request_complete: false,
        stage: 'credential',
      },
    })
    try {
      await act(async () =>
        waitForCondition(
          () =>
            document.querySelector(
              '[data-testid="assistant-onboarding-todo"]'
            ) !== null,
          'L1 onboarding todo did not render'
        )
      )
      assert.match(document.body.textContent ?? '', /First-use checklist/)
      assert.match(document.body.textContent ?? '', /Create API key/)
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
            choices: [
              { message: { content: 'I prepared the exact preview.' } },
            ],
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
      assert.match(
        document.body.textContent ?? '',
        /ADMIN · Administrator mode/
      )
      const textarea = document.querySelector<HTMLTextAreaElement>(
        'textarea[placeholder="Ask about server configuration, model pricing, or operations..."]'
      )
      assert.ok(textarea)
      await setTextareaValue(textarea, 'Turn on the desktop sidebar default.')
      await act(async () => {
        document
          .querySelector<HTMLButtonElement>('button[aria-label="Submit"]')
          ?.click()
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
            document.body.textContent?.includes(
              'Administrator change applied'
            ) === true,
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
            document.body.textContent?.includes(
              'Lower-access user conversation'
            ) === true,
          'Assistant history did not render'
        )
      )
      assert.doesNotMatch(
        document.body.textContent ?? '',
        /private@example\.test/
      )
      assert.doesNotMatch(
        document.body.textContent ?? '',
        /sk-history-secret-123456/
      )

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

  test('redacts mixed sensitive input and continues the assistant request', async () => {
    let postedBody: unknown
    api.get = (async (url: string) => {
      assert.equal(url, '/api/assistant/status')
      return { data: { success: true, data: assistantStatus } }
    }) as typeof api.get
    api.post = (async (url: string, data: unknown) => {
      assert.equal(url, '/api/assistant/chat')
      postedBody = data
      return {
        data: { choices: [{ message: { content: 'Here is the diagnosis.' } }] },
        headers: {},
      }
    }) as typeof api.post

    const rendered = await renderPanel()
    try {
      const textarea = document.querySelector<HTMLTextAreaElement>('textarea')
      assert.ok(textarea)
      await setTextareaValue(
        textarea,
        'Explain this failure for private@example.test with sk-private-secret-123456.'
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
            document.body.textContent?.includes('Here is the diagnosis.') ===
            true,
          'Redacted assistant request did not complete'
        )
      )
      const serializedBody = JSON.stringify(postedBody)
      assert.doesNotMatch(serializedBody, /private@example\.test/)
      assert.doesNotMatch(serializedBody, /sk-private-secret-123456/)
      assert.match(serializedBody, /REDACTED_EMAIL/)
      assert.match(serializedBody, /REDACTED_API_KEY/)
      assert.match(
        document.body.textContent ?? '',
        /Sensitive content was redacted before sending\./
      )
      assert.doesNotMatch(
        document.body.textContent ?? '',
        /private@example\.test/
      )
    } finally {
      await act(async () => rendered.root.unmount())
      rendered.queryClient.clear()
    }
  })

  test('does not send a message that contains only a secret', async () => {
    let posted = 0
    api.get = (async (url: string) => {
      assert.equal(url, '/api/assistant/status')
      return { data: { success: true, data: assistantStatus } }
    }) as typeof api.get
    api.post = (async () => {
      posted += 1
      throw new Error('A secret-only message must not be sent')
    }) as typeof api.post

    const rendered = await renderPanel()
    try {
      const textarea = document.querySelector<HTMLTextAreaElement>('textarea')
      assert.ok(textarea)
      await setTextareaValue(textarea, 'sk-private-secret-123456')
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
        /Only sensitive content remained after redaction\./
      )
      assert.doesNotMatch(
        document.body.textContent ?? '',
        /sk-private-secret-123456/
      )
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

  test('keeps the direct L1 request path available when the AI request fails', async () => {
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
    api.post = (async (url: string) => {
      assert.equal(url, '/api/assistant/chat')
      throw new Error('assistant offline')
    }) as typeof api.post

    const rendered = await renderPanel('onboarding')
    try {
      const textarea = document.querySelector<HTMLTextAreaElement>(
        'textarea[placeholder="Write a short explanation of what you want to build or why you need L1 access."]'
      )
      assert.ok(textarea)
      await setTextareaValue(textarea, 'I need L1 for a small integration.')

      await act(async () => {
        findButton('Send').click()
        await flushEffects()
      })
      await act(async () =>
        waitForCondition(
          () =>
            document.body.textContent?.includes(
              'The AI assistant could not answer right now.'
            ) === true,
          'Assistant failure message did not render'
        )
      )

      assert.ok(
        document.querySelector(
          'textarea[placeholder="Explain what you want to build and why you need L1 access."]'
        )
      )
      assert.ok(findButton('Submit for administrator review'))
    } finally {
      await act(async () => rendered.root.unmount())
      rendered.queryClient.clear()
    }
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
          () =>
            document.body.textContent?.includes('Keep your key private.') ===
            true,
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
