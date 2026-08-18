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
import { after, describe, test } from 'node:test'

import { Window } from 'happy-dom'

const domWindow = new Window()
const domGlobals = [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'HTMLInputElement',
  'SVGElement',
  'Node',
  'Element',
  'Event',
  'CustomEvent',
  'MutationObserver',
  'requestAnimationFrame',
  'cancelAnimationFrame',
  'getComputedStyle',
] as const

for (const key of domGlobals) {
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: domWindow[key],
  })
}

const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { GroupRatioVisualEditor } = await import('../group-ratio-visual-editor')

const i18n = createInstance()
await i18n.use(initReactI18next).init({
  lng: 'en',
  resources: { en: { translation: {} } },
})

const reactTestGlobals = globalThis as typeof globalThis & {
  IS_REACT_ACT_ENVIRONMENT?: boolean
}
reactTestGlobals.IS_REACT_ACT_ENVIRONMENT = true

describe('group ratio visual editor', () => {
  after(() => {
    domWindow.close()
  })

  test('fails closed for an explicit invalid base ratio', async () => {
    const changes: Array<{ field: string; value: string }> = []
    const container = document.createElement('div')
    document.body.append(container)
    const root = createRoot(container)

    await act(async () => {
      root.render(
        <I18nextProvider i18n={i18n}>
          <GroupRatioVisualEditor
            groupRatio='{ "zeta": 1, "beta": 0.5, "alpha": 0.5, "bad": -1 }'
            topupGroupRatio='{}'
            userUsableGroups='{}'
            groupGroupRatio='{}'
            autoGroups='["zeta", "missing", "zeta"]'
            groupSpecialUsableGroup='{}'
            onChange={(field, value) => changes.push({ field, value })}
          />
        </I18nextProvider>
      )
    })

    assert.equal(changes.length, 0)
    const optimizeButton = [...container.querySelectorAll('button')].find(
      (button) => button.textContent === 'Optimize by effective cost'
    )
    assert.ok(optimizeButton)

    assert.equal(optimizeButton.disabled, true)
    assert.equal(changes.length, 0)
    assert.match(
      container.querySelector('[role="alert"]')?.textContent ?? '',
      /Cannot optimize until AutoGroups and ratio maps/
    )

    await act(async () => root.unmount())
    container.remove()
  })

  test('fails closed for malformed UserUsableGroups without crashing', async () => {
    for (const userUsableGroups of ['null', '[]', '{ "vip": 1 }']) {
      const changes: Array<{ field: string; value: string }> = []
      const container = document.createElement('div')
      document.body.append(container)
      const root = createRoot(container)

      await act(async () => {
        root.render(
          <I18nextProvider i18n={i18n}>
            <GroupRatioVisualEditor
              groupRatio='{ "default": 1 }'
              topupGroupRatio='{}'
              userUsableGroups={userUsableGroups}
              groupGroupRatio='{}'
              autoGroups='["default"]'
              groupSpecialUsableGroup='{}'
              onChange={(field, value) => changes.push({ field, value })}
            />
          </I18nextProvider>
        )
      })

      const optimizeButton = [...container.querySelectorAll('button')].find(
        (button) => button.textContent === 'Optimize by effective cost'
      )
      assert.ok(optimizeButton)
      assert.equal(optimizeButton.disabled, true)
      assert.equal(changes.length, 0)
      assert.match(
        container.querySelector('[role="alert"]')?.textContent ?? '',
        /Cannot optimize until AutoGroups and ratio maps/
      )

      await act(async () => root.unmount())
      container.remove()
    }
  })

  test('optimizes registered groups by the base billing ratio on explicit action', async () => {
    const changes: Array<{ field: string; value: string }> = []
    const container = document.createElement('div')
    document.body.append(container)
    const root = createRoot(container)

    await act(async () => {
      root.render(
        <I18nextProvider i18n={i18n}>
          <GroupRatioVisualEditor
            groupRatio='{ "zeta": 1, "beta": 0.5, "alpha": 0.5 }'
            topupGroupRatio='{}'
            userUsableGroups='{}'
            groupGroupRatio='{}'
            autoGroups='["zeta", "missing", "zeta"]'
            groupSpecialUsableGroup='{}'
            onChange={(field, value) => changes.push({ field, value })}
          />
        </I18nextProvider>
      )
    })

    const optimizeButton = [...container.querySelectorAll('button')].find(
      (button) => button.textContent === 'Optimize by effective cost'
    )
    assert.ok(optimizeButton)
    assert.equal(optimizeButton.disabled, false)
    assert.equal(changes.length, 0)

    await act(async () => {
      optimizeButton.click()
    })

    assert.deepEqual(changes, [
      {
        field: 'AutoGroups',
        value: JSON.stringify(['alpha', 'beta', 'zeta'], null, 2),
      },
    ])

    await act(async () => root.unmount())
    container.remove()
  })

  test('uses the selected user group override as the optimization baseline', async () => {
    const changes: Array<{ field: string; value: string }> = []
    const container = document.createElement('div')
    document.body.append(container)
    const root = createRoot(container)

    await act(async () => {
      root.render(
        <I18nextProvider i18n={i18n}>
          <GroupRatioVisualEditor
            groupRatio='{ "alpha": 1, "beta": 0.5, "vip": 1, "zeta": 2 }'
            topupGroupRatio='{}'
            userUsableGroups='{}'
            groupGroupRatio='{ "vip": { "zeta": 0.1, "beta": 1.5 } }'
            autoGroups='["alpha", "beta", "vip", "zeta"]'
            groupSpecialUsableGroup='{}'
            onChange={(field, value) => changes.push({ field, value })}
          />
        </I18nextProvider>
      )
    })

    const selectTriggers = container.querySelectorAll<HTMLElement>(
      '[data-slot="select-trigger"]'
    )
    const baselineTrigger = [...selectTriggers].at(-1)
    assert.ok(baselineTrigger)
    await act(async () => {
      baselineTrigger.click()
    })

    const vipOption = [...document.querySelectorAll('[role="option"]')].find(
      (option) => option.textContent === 'vip'
    ) as HTMLElement | undefined
    assert.ok(vipOption)
    await act(async () => {
      vipOption.click()
    })

    const optimizeButton = [...container.querySelectorAll('button')].find(
      (button) => button.textContent === 'Optimize by effective cost'
    )
    assert.ok(optimizeButton)
    await act(async () => {
      optimizeButton.click()
    })

    assert.deepEqual(changes, [
      {
        field: 'AutoGroups',
        value: JSON.stringify(['zeta', 'alpha', 'vip', 'beta'], null, 2),
      },
    ])

    await act(async () => root.unmount())
    container.remove()
  })

  test('fails closed for an invalid special ratio even with the base baseline', async () => {
    const changes: Array<{ field: string; value: string }> = []
    const container = document.createElement('div')
    document.body.append(container)
    const root = createRoot(container)

    await act(async () => {
      root.render(
        <I18nextProvider i18n={i18n}>
          <GroupRatioVisualEditor
            groupRatio='{ "alpha": 1, "other": 1, "zeta": 2 }'
            topupGroupRatio='{}'
            userUsableGroups='{}'
            groupGroupRatio='{ "other": { "zeta": -1 } }'
            autoGroups='["alpha", "other", "zeta"]'
            groupSpecialUsableGroup='{}'
            onChange={(field, value) => changes.push({ field, value })}
          />
        </I18nextProvider>
      )
    })

    const optimizeButton = [...container.querySelectorAll('button')].find(
      (button) => button.textContent === 'Optimize by effective cost'
    )
    assert.ok(optimizeButton)
    assert.equal(optimizeButton.disabled, true)
    assert.equal(changes.length, 0)
    assert.match(
      container.querySelector('[role="alert"]')?.textContent ?? '',
      /Cannot optimize until AutoGroups and ratio maps/
    )

    await act(async () => root.unmount())
    container.remove()
  })
})
