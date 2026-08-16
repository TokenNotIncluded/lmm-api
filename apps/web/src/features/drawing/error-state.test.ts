/*
Copyright (C) 2026 LIghtJUNction
*/
import assert from 'node:assert/strict'
import { describe, test } from 'node:test'

import {
  getDrawingRequestErrorKind,
  getDrawingRequestStatus,
} from './error-state'

describe('drawing request error state', () => {
  test('does not classify an expired session as an L1 permission denial', () => {
    const error = { response: { status: 401 } }

    assert.equal(getDrawingRequestStatus(error), 401)
    assert.equal(getDrawingRequestErrorKind(error), 'unauthenticated')
  })

  test('keeps an actual forbidden response distinguishable', () => {
    const error = { response: { status: 403 } }

    assert.equal(getDrawingRequestErrorKind(error), 'forbidden')
  })

  test('marks upstream outages and network failures for retry UI', () => {
    assert.equal(
      getDrawingRequestErrorKind({ response: { status: 503 } }),
      'unavailable'
    )
    assert.equal(getDrawingRequestErrorKind(new Error('network')), 'network')
    assert.equal(getDrawingRequestStatus(new Error('network')), null)
  })
})
