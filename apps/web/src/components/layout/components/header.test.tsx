import assert from 'node:assert/strict'
/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.
*/
import { after, test } from 'node:test'

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
  'requestAnimationFrame',
  'cancelAnimationFrame',
] as const) {
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: domWindow[key],
  })
}

Object.defineProperty(window, 'matchMedia', {
  configurable: true,
  value: () => ({
    addEventListener: () => undefined,
    matches: false,
    removeEventListener: () => undefined,
  }),
})

const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { SidebarProvider } = await import('@/components/ui/sidebar')
const { Header } = await import('./header')

const reactTestGlobals = globalThis as typeof globalThis & {
  IS_REACT_ACT_ENVIRONMENT?: boolean
}
reactTestGlobals.IS_REACT_ACT_ENVIRONMENT = true

const i18n = createInstance()
await i18n.use(initReactI18next).init({
  lng: 'en',
  resources: { en: { translation: {} } },
})

after(() => domWindow.close())

async function renderHeader(showSidebarTrigger?: boolean) {
  const container = document.createElement('div')
  const root = createRoot(container)
  await act(async () => {
    root.render(
      <I18nextProvider i18n={i18n}>
        <SidebarProvider>
          <Header showSidebarTrigger={showSidebarTrigger} />
        </SidebarProvider>
      </I18nextProvider>
    )
  })
  return { container, root }
}

test('Header shows the sidebar trigger by default', async () => {
  const rendered = await renderHeader()
  try {
    assert.ok(rendered.container.querySelector('[data-sidebar="trigger"]'))
  } finally {
    await act(async () => rendered.root.unmount())
  }
})

test('Header can hide the trigger when the layout has no sidebar', async () => {
  const rendered = await renderHeader(false)
  try {
    assert.equal(
      rendered.container.querySelector('[data-sidebar="trigger"]'),
      null
    )
  } finally {
    await act(async () => rendered.root.unmount())
  }
})
