/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.
*/
import assert from 'node:assert/strict'
import { describe, test } from 'node:test'

import {
  securityAuditTotalPages,
  securityAuditUserFilter,
} from './security-audit-utils'

describe('security audit utilities', () => {
  test('keeps every active audit lane reachable from the unfiltered pager', () => {
    assert.equal(
      securityAuditTotalPages({
        deterministicTotal: 1,
        aiReviewTotal: 30,
        pageSize: 20,
      }),
      2
    )
    assert.equal(
      securityAuditTotalPages({
        deterministicTotal: 41,
        aiReviewTotal: 20,
        pageSize: 20,
      }),
      3
    )
  })

  test('uses the selected audit lane when a source filter is active', () => {
    assert.equal(
      securityAuditTotalPages({
        source: 'ai_review',
        deterministicTotal: 90,
        aiReviewTotal: 21,
        pageSize: 20,
      }),
      2
    )
    assert.equal(
      securityAuditTotalPages({
        source: 'deterministic_rule',
        deterministicTotal: 21,
        aiReviewTotal: 90,
        pageSize: 20,
      }),
      2
    )
  })

  test('only builds a user-management filter from an authorized identity', () => {
    assert.equal(
      securityAuditUserFilter({ username: '  audit-user  ', user_id: 42 }),
      'audit-user'
    )
    assert.equal(securityAuditUserFilter({ user_id: 42 }), '42')
    assert.equal(
      securityAuditUserFilter({ user_id: 0, username: '   ' }),
      undefined
    )
  })
})
