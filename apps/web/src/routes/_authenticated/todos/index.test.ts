/*
Copyright (C) 2026 LIghtJUNction
*/
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { todosSearchSchema } from './index'

test('keeps a developer-access request target in to-do navigation state', () => {
  assert.deepEqual(
    todosSearchSchema.parse({ todo: 'developer_access', request: 42 }),
    { todo: 'developer_access', request: 42 }
  )
})
