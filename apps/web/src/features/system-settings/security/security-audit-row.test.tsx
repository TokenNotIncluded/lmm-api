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
  'HTMLDetailsElement',
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

const { act, createElement } = await import('react')
const { createRoot } = await import('react-dom/client')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { AuditRow } = await import('./security-audit')

const reactTestGlobals = globalThis as typeof globalThis & {
  IS_REACT_ACT_ENVIRONMENT?: boolean
}
reactTestGlobals.IS_REACT_ACT_ENVIRONMENT = true

const i18n = createInstance()
await i18n.use(initReactI18next).init({
  lng: 'en',
  resources: { en: { translation: {} } },
})

async function renderRow() {
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  await act(async () => {
    root.render(
      createElement(
        I18nextProvider,
        { i18n },
        createElement(AuditRow, {
          event: {
            id: 1,
            created_at: 1_786_400_000,
            request_id: 'req_1234567890abcdef',
            source: 'ai_review',
            decision: 'violation',
            rule_id: 'abuse.policy',
            rule_version: '2026-08',
            endpoint: '/v1/responses',
            review_model: 'deepseek-v4-flash',
            explanation:
              'The request matched the configured abuse policy and requires administrator review.',
          },
        })
      )
    )
  })
  return { container, root }
}

afterEach(() => document.body.replaceChildren())
after(() => domWindow.close())

describe('AuditRow details', () => {
  test('keeps the row compact while exposing complete authorized audit metadata', async () => {
    const rendered = await renderRow()
    try {
      const details = rendered.container.querySelector('details')
      assert.ok(details)
      assert.equal(details.open, false)
      assert.match(details.textContent ?? '', /View details/)

      const summary = details.querySelector('summary')
      assert.ok(summary)
      await act(async () => {
        summary.click()
      })

      assert.equal(details.open, true)
      assert.match(details.textContent ?? '', /req_1234567890abcdef/)
      assert.match(details.textContent ?? '', /abuse\.policy · 2026-08/)
      assert.match(details.textContent ?? '', /\/v1\/responses/)
      assert.match(details.textContent ?? '', /deepseek-v4-flash/)
      assert.match(details.textContent ?? '', /requires administrator review/)
    } finally {
      await act(async () => rendered.root.unmount())
      rendered.container.remove()
    }
  })
})
