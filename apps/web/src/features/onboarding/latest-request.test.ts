/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { afterEach, beforeEach, test } from 'node:test'

import { usageLogSchema } from '@/features/usage-logs/data/schema'
import { api } from '@/lib/api'
import { useAuthStore } from '@/stores/auth-store'

import { latestRequestStatus, loadLatestRequest } from './latest-request'

const originalGet = api.get
const originalAuth = useAuthStore.getState().auth
const fixture = (type: number, created_at: number, other = '') =>
  usageLogSchema.parse({
    id: type,
    user_id: 1,
    type,
    created_at,
    content: '',
    other,
  })
beforeEach(() => {
  useAuthStore
    .getState()
    .auth.setUser({ id: 1, username: 'synthetic', role: 1 })
})
afterEach(() => {
  api.get = originalGet
  useAuthStore.setState({ auth: originalAuth })
})
test('selects the latest error or usage entry instead of the latest payment', async () => {
  api.get = (async (path: string) => ({
    data: {
      success: true,
      data: {
        page: 1,
        page_size: 1,
        total: 1,
        items: [path.includes('type=5') ? fixture(5, 200) : fixture(2, 100)],
      },
    },
  })) as typeof api.get
  assert.equal((await loadLatestRequest())?.type, 5)
})
test('does not turn a failed log query into an empty request history', async () => {
  api.get = (async () => ({ data: { success: false } })) as typeof api.get
  await assert.rejects(loadLatestRequest())
})
test('usage accounting alone is not evidence of a successful response', () => {
  assert.equal(latestRequestStatus(fixture(2, 100)), 'recorded')
  assert.equal(
    latestRequestStatus(fixture(2, 100, '{"acquisition_success_v1":true}')),
    'successful'
  )
  assert.equal(
    latestRequestStatus(
      fixture(
        2,
        100,
        '{"acquisition_success_v1":true,"stream_status":{"status":"error"}}'
      )
    ),
    'failed'
  )
  assert.equal(latestRequestStatus(fixture(5, 100)), 'failed')
})

test('native incomplete type-five receipts are not failed or successful responses', () => {
  for (const state of ['reserved', 'streaming', 'reconcile', 'released']) {
    const log = usageLogSchema.parse({
      ...fixture(5, 100),
      quota: null,
      billing_basis: 'incomplete',
      state,
      other: JSON.stringify({
        billing_basis: 'incomplete',
        request_state: state,
        usage_status: 'incomplete',
        reserved_credits: state === 'released' ? 0 : 2000,
        acquisition_success_v1: true,
        stream_status: { status: 'error' },
      }),
    })
    assert.equal(
      latestRequestStatus(log),
      state === 'reconcile' ? 'unknown' : 'incomplete'
    )
    assert.equal(log.type, 5)
    assert.equal(log.quota, null)
  }
})

test('settled type-five error outcome retains its failure display', () => {
  const log = usageLogSchema.parse({
    ...fixture(5, 100),
    quota: 7,
    billing_basis: 'net_consumption',
    state: 'settled',
    other: JSON.stringify({
      billing_basis: 'net_consumption',
      request_state: 'settled',
      usage_status: 'settled',
    }),
  })
  assert.equal(latestRequestStatus(log), 'failed')
})
