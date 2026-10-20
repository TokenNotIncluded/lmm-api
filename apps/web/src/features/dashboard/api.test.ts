/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { afterEach, test } from 'node:test'

import { api } from '@/lib/api'

import { getUserQuotaDates } from './api'

const originalGet = api.get
afterEach(() => {
  api.get = originalGet
})
const params = { start_timestamp: 0, end_timestamp: 10 }

test('dashboard validates bucket data and keeps explicit successful legacy null empty', async () => {
  api.get = (async () => ({
    data: { success: true, data: null },
  })) as typeof api.get
  assert.deepEqual(await getUserQuotaDates(params), { success: true, data: [] })
  api.get = (async () => ({
    data: {
      success: true,
      data: [{ created_at: 1, token_used: null, quota: 10 }],
    },
  })) as typeof api.get
  assert.equal((await getUserQuotaDates(params)).data[0].token_used, null)
})

test('failed, missing and malformed payloads never become successful empty usage', async () => {
  for (const data of [
    { success: false, data: [] },
    { success: true },
    { success: true, data: [{ created_at: '1' }] },
    {
      success: true,
      data: [{ created_at: 1, token_count_scope: 'settled_text_requests' }],
    },
  ]) {
    api.get = (async () => ({ data })) as typeof api.get
    await assert.rejects(getUserQuotaDates(params))
  }
})
