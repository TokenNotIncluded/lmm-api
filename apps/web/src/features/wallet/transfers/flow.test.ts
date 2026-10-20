/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { afterEach, test } from 'node:test'

import { api } from '@/lib/api'

import {
  cancelTransfer,
  createTransfer,
  queryTransfer,
  queryTransferByID,
} from './api'
import {
  canShareTransfer,
  hasRecipientDetails,
  reconcileTransfer,
  trustedConfirmationURL,
  transferSchema,
  type PendingTransfer,
} from './flow'

const row = transferSchema.parse({
  id: 17,
  quota: 125,
  status: 'awaiting_confirmation',
  created_at: 1,
  request_key: 'immutable-wallet-key',
})
const pending: PendingTransfer = {
  action: 'create',
  requestKey: 'immutable-wallet-key',
  quota: 125,
  step: { kind: 'unknown' },
}
const originalPost = api.post,
  originalGet = api.get
afterEach(() => {
  api.post = originalPost
  api.get = originalGet
})
test('only exact configured Core confirmation URL may open', () => {
  const url = 'https://core.example/core/v1/wallet/confirm/create/17'
  assert.equal(
    trustedConfirmationURL(
      url,
      'create',
      17,
      'https://web.example',
      'https://core.example'
    ),
    url
  )
  for (const raw of [
    `${url}?evil`,
    `${url}#evil`,
    url.replace('/17', '/18'),
    url.replace('create', 'cancel'),
    url.replace('core.example', 'evil.example'),
    url.replace('https:', 'http:'),
    'javascript:alert(1)',
    'https://user@core.example/core/v1/wallet/confirm/create/17',
  ]) {
    assert.equal(
      trustedConfirmationURL(
        raw,
        'create',
        17,
        'https://web.example',
        'https://core.example'
      ),
      null
    )
  }
  const local = 'http://127.0.0.1:8080/core/v1/wallet/confirm/create/17'
  assert.equal(
    trustedConfirmationURL(
      local,
      'create',
      17,
      'http://127.0.0.1:3000',
      'http://127.0.0.1:8080'
    ),
    local
  )
})
test('confirmation preserves original key; only actual state resolves operation', () => {
  const result = reconcileTransfer(pending, {
    kind: 'confirmation',
    confirmationURL: 'https://core.example/core/v1/wallet/confirm/create/17',
    transfer: row,
  })
  assert.equal(result.kind, 'pending')
  if (result.kind === 'pending') {
    assert.equal(result.pending.requestKey, pending.requestKey)
    assert.equal(result.pending.step.kind, 'confirmation')
  }
  assert.equal(
    reconcileTransfer(pending, { kind: 'transfer', transfer: row }).kind,
    'pending'
  )
  assert.equal(
    reconcileTransfer(pending, {
      kind: 'transfer',
      transfer: { ...row, status: 'pending' },
    }).kind,
    'complete'
  )
  assert.throws(() =>
    reconcileTransfer(pending, {
      kind: 'transfer',
      transfer: { ...row, quota: 126 },
    })
  )
  const cancel: PendingTransfer = {
    action: 'cancel',
    id: 17,
    quota: 125,
    step: { kind: 'unknown' },
  }
  assert.equal(
    reconcileTransfer(cancel, {
      kind: 'transfer',
      transfer: { ...row, status: 'pending' },
    }).kind,
    'pending'
  )
  assert.equal(
    reconcileTransfer(cancel, {
      kind: 'transfer',
      transfer: { ...row, status: 'cancelled' },
    }).kind,
    'complete'
  )
  assert.equal(
    reconcileTransfer(cancel, {
      kind: 'transfer',
      transfer: { ...row, status: 'claimed' },
    }).kind,
    'conflict'
  )
})
test('unavailable link and recipient flags hide even stray payload fields', () => {
  const value = { ...row, status: 'pending' as const, token: 'a'.repeat(64) }
  assert.equal(canShareTransfer(value), true)
  assert.equal(
    canShareTransfer({ ...value, claim_link_available: false }),
    false
  )
  assert.equal(canShareTransfer({ ...value, token: undefined }), false)
  assert.equal(
    canShareTransfer({ ...value, status: 'awaiting_confirmation' }),
    false
  )
  assert.equal(
    hasRecipientDetails({ ...value, recipient_id: 5, recipient_name: 'name' }),
    true
  )
  assert.equal(
    hasRecipientDetails({
      ...value,
      recipient_id: 5,
      recipient_name: 'name',
      recipient_details_available: false,
    }),
    false
  )
  assert.equal(hasRecipientDetails(value), false)
})
test('202 is confirmation; create POST disables authentication replay', async () => {
  api.post = (async (
    url: string,
    body: unknown,
    options: Record<string, unknown>
  ) => {
    assert.equal(url, '/api/wallet-transfer')
    assert.deepEqual(body, { quota: 125, request_key: row.request_key })
    assert.equal(options.skipAuthRefresh, true)
    return {
      data: {
        success: false,
        code: 'WALLET_CONFIRMATION_REQUIRED',
        data: {
          request_key: row.request_key,
          transfer: row,
          confirmation_url:
            'https://core.example/core/v1/wallet/confirm/create/17',
        },
      },
    }
  }) as typeof api.post
  assert.equal(
    (await createTransfer(125, 'immutable-wallet-key')).kind,
    'confirmation'
  )
})
test('lost create ACK only queries exact original key, never dispatches again', async () => {
  let writes = 0
  api.post = (async () => {
    writes++
    throw new Error('lost ACK')
  }) as typeof api.post
  await assert.rejects(createTransfer(125, 'immutable-wallet-key'))
  api.get = (async (url: string, options: Record<string, unknown>) => {
    assert.equal(url, '/api/wallet-transfer/query')
    assert.deepEqual(options.params, { request_key: row.request_key })
    return { data: { success: true, data: { ...row, status: 'pending' } } }
  }) as typeof api.get
  assert.equal(
    (await queryTransfer('immutable-wallet-key')).transfer.status,
    'pending'
  )
  assert.equal(writes, 1)
  api.get = (async () => ({
    data: { success: false, code: 'WALLET_RESULT_UNKNOWN' },
  })) as typeof api.get
  await assert.rejects(queryTransfer('immutable-wallet-key'))
  assert.equal(writes, 1)
})
test('lost cancel ACK only queries original ID and pending is still confirmation', async () => {
  let writes = 0
  api.post = (async () => {
    writes++
    throw new Error('lost ACK')
  }) as typeof api.post
  await assert.rejects(cancelTransfer(17, 125))
  api.get = (async (url: string, options: Record<string, unknown>) => {
    assert.equal(url, '/api/wallet-transfer/query')
    assert.deepEqual(options.params, { cancel_transfer_id: 17 })
    return {
      data: {
        success: false,
        code: 'WALLET_CONFIRMATION_REQUIRED',
        data: {
          request_key: row.request_key,
          transfer: { ...row, status: 'pending' },
          confirmation_url:
            'https://core.example/core/v1/wallet/confirm/cancel/17',
        },
      },
    }
  }) as typeof api.get
  assert.equal((await queryTransferByID(17)).kind, 'confirmation')
  assert.equal(writes, 1)
})
