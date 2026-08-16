/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.
*/
import assert from 'node:assert/strict'
import { describe, test } from 'node:test'

import {
  getDiscountCodeAvailability,
  parseDiscountCodeMaxUses,
} from './availability'

const now = 1_786_400_000

describe('discount code availability', () => {
  test('distinguishes enabled codes from codes that cannot currently apply', () => {
    assert.equal(
      getDiscountCodeAvailability(
        { status: 1, starts_time: 0, expired_time: 0 },
        now
      ),
      'active'
    )
    assert.equal(
      getDiscountCodeAvailability(
        { status: 1, starts_time: now + 1, expired_time: 0 },
        now
      ),
      'not_started'
    )
    assert.equal(
      getDiscountCodeAvailability(
        { status: 1, starts_time: 0, expired_time: now },
        now
      ),
      'active'
    )
    assert.equal(
      getDiscountCodeAvailability(
        { status: 1, starts_time: 0, expired_time: now - 1 },
        now
      ),
      'expired'
    )
    assert.equal(
      getDiscountCodeAvailability(
        { status: 2, starts_time: 0, expired_time: 0 },
        now
      ),
      'disabled'
    )
  })

  test('accepts only safe whole-number usage limits', () => {
    assert.equal(parseDiscountCodeMaxUses('0'), 0)
    assert.equal(parseDiscountCodeMaxUses('25'), 25)
    assert.equal(parseDiscountCodeMaxUses(' 3 '), 3)
    assert.equal(parseDiscountCodeMaxUses(''), undefined)
    assert.equal(parseDiscountCodeMaxUses('-1'), undefined)
    assert.equal(parseDiscountCodeMaxUses('1.5'), undefined)
    assert.equal(parseDiscountCodeMaxUses('9007199254740992'), undefined)
  })
})
