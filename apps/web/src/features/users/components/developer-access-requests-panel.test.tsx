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
const { api } = await import('@/lib/api')
const { DeveloperAccessRequestsPanel } =
  await import('./developer-access-requests-panel')

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
  document.body.replaceChildren()
})

after(() => domWindow.close())

describe('DeveloperAccessRequestsPanel', () => {
  test('requires an administrator reply before reviewing each user recommendation letter', async () => {
    api.get = (async (url: string) => {
      assert.equal(url, '/api/developer-access/requests')
      return {
        data: {
          success: true,
          data: [
            {
              id: 17,
              user_id: 8,
              status: 'pending',
              reason: 'I will use Claude Code for private development.',
              source: 'assistant_recommendation',
              ai_recommendation:
                'Recommend L1 because the user supplied a concrete use case.',
              admin_user_id: 0,
              admin_note: '',
              created_at: 1_786_400_000,
              reviewed_at: 0,
              username: 'test-user',
              email: 'test@example.test',
            },
            {
              id: 18,
              user_id: 9,
              status: 'pending',
              reason: 'I want to connect a local test client.',
              source: 'assistant_request',
              ai_recommendation: '',
              admin_user_id: 0,
              admin_note: '',
              created_at: 1_786_400_001,
              reviewed_at: 0,
              username: 'manual-user',
              email: 'manual@example.test',
            },
          ],
        },
      }
    }) as typeof api.get

    let reviewRequest: { url: string; data: unknown } | undefined
    api.post = (async (url: string, data: unknown) => {
      reviewRequest = { url, data }
      return { data: { success: true, data: {} } }
    }) as typeof api.post

    const container = document.createElement('div')
    document.body.append(container)
    const root = createRoot(container)
    await act(async () => {
      root.render(
        <I18nextProvider i18n={i18n}>
          <DeveloperAccessRequestsPanel />
        </I18nextProvider>
      )
      await flushEffects()
    })

    try {
      await waitForCondition(
        () => document.body.textContent?.includes('test-user') === true,
        'AI recommendation did not render'
      )
      assert.match(document.body.textContent ?? '', /L1 access requests/)
      assert.match(
        document.body.textContent ?? '',
        /Recommend L1 because the user supplied a concrete use case\./
      )
      assert.match(document.body.textContent ?? '', /manual-user/)
      assert.match(document.body.textContent ?? '', /Direct request/)
      assert.match(
        document.body.textContent ?? '',
        /I want to connect a local test client\./
      )

      const textarea = document.querySelector('textarea')
      assert.ok(textarea)
      const approve = [...document.querySelectorAll('button')].find((button) =>
        button.textContent?.includes('Approve and unlock L1')
      )
      const reject = [...document.querySelectorAll('button')].find((button) =>
        button.textContent?.includes('Reject')
      )
      assert.ok(approve)
      assert.ok(reject)
      assert.equal(approve.disabled, true)
      assert.equal(reject.disabled, true)

      await setTextareaValue(textarea, 'O')
      assert.equal(approve.disabled, true)
      await setTextareaValue(textarea, ' Approved ')
      assert.equal(approve.disabled, false)
      assert.equal(reject.disabled, false)

      await act(async () => {
        approve.click()
        await flushEffects()
      })
      await waitForCondition(
        () => reviewRequest !== undefined,
        'Approval was not submitted'
      )
      assert.deepEqual(reviewRequest, {
        url: '/api/developer-access/requests/17/approve',
        data: { note: 'Approved' },
      })
    } finally {
      await act(async () => root.unmount())
      container.remove()
    }
  })
})
