/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.
*/
import assert from 'node:assert/strict'
import { describe, test } from 'node:test'

import {
  isSidebarRouteEnabledByModules,
  parseSidebarUserSettings,
  resolveSidebarDefaultRoute,
  serializeSidebarUserSettings,
} from '../sidebar-preferences'

describe('sidebar preferences', () => {
  test('keeps legacy module settings readable', () => {
    const settings = parseSidebarUserSettings(
      JSON.stringify({ console: { enabled: true, token: false } })
    )

    assert.deepEqual(settings?.modules.console, {
      enabled: true,
      token: false,
    })
    assert.equal(settings?.preferences.density, 'comfortable')
  })

  test('normalizes the versioned envelope and removes duplicate order entries', () => {
    const settings = parseSidebarUserSettings(
      JSON.stringify({
        modules: { personal: { enabled: true } },
        preferences: {
          section_order: ['personal', 'personal', 42],
          density: 'compact',
          default_route: '/wallet',
        },
      })
    )

    assert.deepEqual(settings?.preferences.section_order, ['personal'])
    assert.equal(settings?.preferences.density, 'compact')
    assert.equal(settings?.preferences.default_route, '/wallet')
  })

  test('accepts only internal, visible landing routes', () => {
    const valid = serializeSidebarUserSettings(
      {},
      {
        section_order: [],
        module_order: {},
        hidden_sections: [],
        hidden: [],
        density: 'comfortable',
        default_route: '/wallet',
      }
    )
    const hidden = serializeSidebarUserSettings(
      {},
      {
        section_order: [],
        module_order: {},
        hidden_sections: ['personal'],
        hidden: [],
        density: 'comfortable',
        default_route: '/wallet',
      }
    )

    assert.equal(resolveSidebarDefaultRoute(valid), '/wallet')
    assert.equal(resolveSidebarDefaultRoute(hidden), '/dashboard')
    assert.equal(
      resolveSidebarDefaultRoute(
        serializeSidebarUserSettings(
          {},
          {
            section_order: [],
            module_order: {},
            hidden_sections: [],
            hidden: [],
            density: 'comfortable',
            default_route: 'https://example.com',
          }
        )
      ),
      '/dashboard'
    )
  })

  test('uses explicit false as the only module-layer visibility override', () => {
    assert.equal(
      isSidebarRouteEnabledByModules('/dashboard/overview', {
        console: { enabled: true, detail: false },
      }),
      false
    )
    assert.equal(
      isSidebarRouteEnabledByModules('/dashboard/overview', {
        console: { enabled: true },
      }),
      true
    )
    assert.equal(isSidebarRouteEnabledByModules('/profile', '{bad json'), true)
  })
})
