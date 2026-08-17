/*
Copyright (C) 2026 LIghtJUNction
*/
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { describe, test } from 'node:test'

const source = readFileSync(
  new URL('./subscription-plans-card.tsx', import.meta.url),
  'utf8'
)

describe('subscription controls on mobile', () => {
  test('keeps billing preference controls reachable at 390px', () => {
    assert.match(source, /h-11 flex-1 text-xs sm:h-8/)
    assert.match(source, /className='size-11 sm:size-8'/)
  })
})
