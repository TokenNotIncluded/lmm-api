/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.
*/
import assert from 'node:assert/strict'
import { describe, test } from 'node:test'

import { extractStatusData } from './api'

describe('public status capability payload', () => {
  test('accepts a structured capability payload', () => {
    assert.deepEqual(extractStatusData({ data: { register_enabled: true } }), {
      register_enabled: true,
    })
  })

  test('fails instead of leaving registration loading forever when data is absent', () => {
    assert.throws(
      () => extractStatusData({ success: false, message: 'not ready' }),
      /Status response did not include capability data/
    )
    assert.throws(
      () => extractStatusData(null),
      /Status response did not include capability data/
    )
  })
})
