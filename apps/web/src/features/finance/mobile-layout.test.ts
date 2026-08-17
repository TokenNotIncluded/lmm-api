/*
Copyright (C) 2026 LIghtJUNction
*/
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { describe, test } from 'node:test'

const source = readFileSync(new URL('./index.tsx', import.meta.url), 'utf8')

describe('finance mobile payment methods', () => {
  test('moves controls below the method details on narrow screens', () => {
    assert.match(
      source,
      /flex flex-wrap items-center gap-3 py-3 sm:flex-nowrap/
    )
    assert.match(source, /w-full flex-wrap items-center gap-x-3 gap-y-1 pl-7/)
    assert.match(
      source,
      /min-h-11 items-center gap-2 whitespace-nowrap sm:min-h-0/
    )
  })
})
