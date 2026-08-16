/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
GNU Affero General Public License for more details.
*/
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { describe, test } from 'node:test'

const source = readFileSync(
  new URL('./use-sidebar-data.ts', import.meta.url),
  'utf8'
)

describe('authenticated sidebar discovery', () => {
  test('keeps the model square reachable from the activated mobile sidebar', () => {
    const generalStart = source.indexOf("id: 'general'")
    const personalStart = source.indexOf("id: 'personal'", generalStart)
    const activatedGeneralSection =
      generalStart >= 0 && personalStart > generalStart
        ? source.slice(generalStart, personalStart)
        : ''

    assert.ok(activatedGeneralSection)
    assert.match(activatedGeneralSection, /title: t\('Model Square'\)/)
    assert.match(activatedGeneralSection, /url: '\/pricing'/)
  })
})
