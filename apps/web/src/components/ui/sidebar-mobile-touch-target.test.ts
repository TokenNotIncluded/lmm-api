/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.
*/
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { describe, test } from 'node:test'

const source = readFileSync(new URL('./sidebar.tsx', import.meta.url), 'utf8')

describe('sidebar mobile navigation', () => {
  test('keeps primary sidebar entries reachable on narrow screens', () => {
    assert.match(
      source,
      /'peer\/menu-button[^']*min-h-11[^']*\[&_svg\]:size-4[^']*sm:min-h-8/
    )
  })
})
