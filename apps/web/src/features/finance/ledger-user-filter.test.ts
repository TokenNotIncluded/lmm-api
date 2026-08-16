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
  financeLedgerUserFilter,
  financeLedgerUserSearch,
} from './ledger-user-filter'

describe('finance ledger user filter', () => {
  test('only pre-fills the ledger with a positive integer user id', () => {
    assert.equal(financeLedgerUserFilter(42), '42')
    assert.equal(financeLedgerUserFilter(), '')
    assert.equal(financeLedgerUserFilter(0), '')
    assert.equal(financeLedgerUserFilter(-1), '')
    assert.equal(financeLedgerUserFilter(1.5), '')
  })

  test('opens the user audit without the default L0-only restriction', () => {
    assert.deepEqual(financeLedgerUserSearch(42), {
      page: 1,
      pageSize: undefined,
      filter: '42',
      status: [],
      role: [],
      group: '',
      l0Only: false,
    })
    assert.equal(financeLedgerUserSearch(0), undefined)
  })
})
