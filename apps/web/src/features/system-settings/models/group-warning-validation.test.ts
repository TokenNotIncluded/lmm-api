/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.
*/
import assert from 'node:assert/strict'
import { describe, test } from 'node:test'

import { isValidGroupWarnings } from './group-warning-validation'

describe('group warning validation', () => {
  test('accepts the server-compatible warning shape', () => {
    assert.equal(
      isValidGroupWarnings({
        free: {
          enabled: true,
          message: 'Community-operated routing. Do not send secrets.',
          mode: 'modal',
          confirmations: 3,
        },
        default: { enabled: false },
      }),
      true
    )
  })

  test('rejects incomplete or invalid warning structures before save', () => {
    for (const value of [
      null,
      [],
      { free: { enabled: true } },
      { free: { enabled: true, message: 42 } },
      { free: { mode: 'toast' } },
      { free: { confirmations: 4 } },
      { ' ': { enabled: false } },
    ]) {
      assert.equal(isValidGroupWarnings(value), false)
    }
  })
})
