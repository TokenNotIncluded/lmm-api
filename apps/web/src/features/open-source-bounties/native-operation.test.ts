// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
import assert from 'node:assert/strict'
import { test } from 'node:test'

import {
  activateBountyAccount,
  beginBountyOperation,
  bountyOperationsSnapshot,
  completeBountyOperation,
  hasBountyOperation,
  pendingBountyOperation,
  settleBountyOperation,
  trustedBountyConfirmationURL,
  waitForBountyOperation,
} from './native-operation'

test('only an exact original confirmation path on configured Core origin opens', () => {
  const path = '/core/v1/bounties/confirm/original-confirmation_ID'
  assert.equal(
    trustedBountyConfirmationURL(
      path,
      'http://127.0.0.1:19001',
      'http://127.0.0.1:19000'
    ),
    `http://127.0.0.1:19000${path}`
  )
  assert.equal(
    trustedBountyConfirmationURL(path, 'https://web.example'),
    `https://web.example${path}`
  )
  for (const value of [
    `https://evil.example${path}`,
    `//evil.example${path}`,
    `${path}?redirect=https://evil.example`,
    `${path}#evil`,
    '/core/v1/bounties/confirm/../other',
    '/core/v1/bounties/confirm/a%2Fb',
  ]) {
    assert.equal(
      trustedBountyConfirmationURL(
        value,
        'https://web.example',
        'https://core.example'
      ),
      null
    )
  }
  for (const origin of [
    'https://core.example/path',
    'https://user:secret@core.example',
    'javascript:evil',
    'https://core.example/',
  ]) {
    assert.equal(
      trustedBountyConfirmationURL(path, 'https://web.example', origin),
      null
    )
  }
})

test('pending operation preserves hash IDs, blocks a new action and remains until business result is read', async () => {
  activateBountyAccount('controller-test-owner')
  const id = 'original-op-hash_1'
  beginBountyOperation(id)
  pendingBountyOperation({
    operation_id: id,
    state: 'awaiting_confirmation',
    confirmation_url: '/core/v1/bounties/confirm/original-confirmation',
  })
  assert.deepEqual(bountyOperationsSnapshot(), [
    {
      id,
      phase: 'confirmation',
      url: '/core/v1/bounties/confirm/original-confirmation',
    },
  ])
  assert.throws(() => beginBountyOperation('new-operation'), /already pending/)
  assert.equal(beginBountyOperation(id), false)
  assert.throws(
    () =>
      pendingBountyOperation(
        { operation_id: 'other-operation', state: 'confirmed' },
        id
      ),
    /mismatch/
  )
  assert.equal(bountyOperationsSnapshot()[0]?.id, id)
  const waiter = waitForBountyOperation(id)
  assert.equal(settleBountyOperation(id), true)
  await waiter
  assert.equal(hasBountyOperation(id), true)
  completeBountyOperation(id)
  assert.equal(hasBountyOperation(id), false)
})

test('unknown recovery keeps the original ID and rejects hostile confirmation links', () => {
  activateBountyAccount('controller-test-recovery')
  beginBountyOperation('persisted-hash')
  pendingBountyOperation({ operation_id: 'persisted-hash', state: 'unknown' })
  assert.equal(bountyOperationsSnapshot()[0]?.phase, 'unknown')
  assert.throws(() =>
    pendingBountyOperation({
      operation_id: 'persisted-hash',
      state: 'confirmed',
      confirmation_url: 'https://evil.example/core/v1/bounties/confirm/fake',
    })
  )
  assert.equal(bountyOperationsSnapshot()[0]?.id, 'persisted-hash')
  completeBountyOperation('persisted-hash')
})

test('expiration rejects the waiting action and removes it without a resubmission', async () => {
  activateBountyAccount('controller-test-expired')
  beginBountyOperation('expired-operation')
  const wait = waitForBountyOperation('expired-operation')
  const rejection = assert.rejects(wait, /Confirmation expired/)
  completeBountyOperation(
    'expired-operation',
    new Error('Confirmation expired')
  )
  await rejection
  assert.equal(hasBountyOperation('expired-operation'), false)
})
