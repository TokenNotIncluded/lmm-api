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

const source = readFileSync(
  new URL('./forge-home.tsx', import.meta.url),
  'utf8'
)

describe('Forge home mobile controls', () => {
  test('keeps the assistant submit target at 44px on narrow screens', () => {
    assert.match(
      source,
      /<InputGroupButton[\s\S]*className='h-11 rounded-lg px-3 sm:h-10'/
    )
  })
})
