/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.
*/
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { describe, test } from 'node:test'

const source = readFileSync(
  new URL('./search-bar.tsx', import.meta.url),
  'utf8'
)

describe('model square search', () => {
  test('keeps the mobile clear-search target reachable', () => {
    assert.match(source, /size-11 sm:size-7/)
  })
})
