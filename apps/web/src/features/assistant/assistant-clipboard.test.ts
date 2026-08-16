/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.
*/
import assert from 'node:assert/strict'
import { describe, test } from 'node:test'

import { copyAssistantText } from './assistant-clipboard'

describe('assistant clipboard', () => {
  test('only reports success after the platform clipboard resolves', async () => {
    const copied: string[] = []
    assert.equal(
      await copyAssistantText('WEEKLY-20', {
        writeText: async (value) => {
          copied.push(value)
        },
      }),
      true
    )
    assert.deepEqual(copied, ['WEEKLY-20'])
  })

  test('reports failure when copying is unavailable or rejected', async () => {
    assert.equal(await copyAssistantText('WEEKLY-20', undefined), false)
    assert.equal(
      await copyAssistantText('WEEKLY-20', {
        writeText: async () => Promise.reject(new Error('denied')),
      }),
      false
    )
  })
})
