/*
Copyright (C) 2026 LIghtJUNction
*/
import assert from 'node:assert/strict'
import { describe, test } from 'node:test'

import { financeChartState } from './chart-state'

describe('finance chart state', () => {
  test('prioritizes errors before loading or empty data', () => {
    assert.equal(
      financeChartState({ hasError: true, isLoading: true, pointCount: 0 }),
      'error'
    )
    assert.equal(
      financeChartState({ hasError: true, isLoading: false, pointCount: 3 }),
      'error'
    )
  })

  test('distinguishes loading, empty, and ready data', () => {
    assert.equal(
      financeChartState({ hasError: false, isLoading: true, pointCount: 0 }),
      'loading'
    )
    assert.equal(
      financeChartState({ hasError: false, isLoading: false, pointCount: 0 }),
      'empty'
    )
    assert.equal(
      financeChartState({ hasError: false, isLoading: false, pointCount: 1 }),
      'ready'
    )
  })
})
