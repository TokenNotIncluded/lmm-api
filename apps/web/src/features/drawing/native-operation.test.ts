/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import {
  clearDrawingOperation,
  imageConfirmationURL,
  readDrawingOperation,
  referenceImageMetadata,
  saveDrawingOperation,
  settledImageSchema,
  type DrawingOperation,
} from './native-operation'

const operation: DrawingOperation = {
  requestId: '0487481e-4fc1-455f-bf81-b3f710a5ac52',
  userId: 79214,
  toolId: 'tool',
  versionId: 'version',
  grantId: 'grant',
  prompt: 'Test',
  model: 'image-test',
  group: 'images',
  count: 1,
  referenceCount: 0,
  createdAt: 1,
}

test('persists the original operation identity and only clears the same request', () => {
  saveDrawingOperation(operation)
  assert.deepEqual(readDrawingOperation(operation.userId), operation)
  assert.equal(readDrawingOperation(operation.userId + 1), null)
  clearDrawingOperation(operation.userId, 'different-request')
  assert.equal(
    readDrawingOperation(operation.userId)?.requestId,
    operation.requestId
  )
  clearDrawingOperation(operation.userId, operation.requestId)
  assert.equal(readDrawingOperation(operation.userId), null)
})

test('only links to the same Core image confirmation browser page', () => {
  const origin = 'https://local.example'
  const imageID = `image:${'a'.repeat(64)}`
  const imagePath = `/api/user/auth/images/confirm/${imageID}`
  for (const confirmationURL of [imagePath, `${origin}${imagePath}`]) {
    assert.equal(
      imageConfirmationURL({ confirmation_url: confirmationURL }, origin),
      `${origin}${imagePath}`
    )
  }
  for (const url of [
    `https://provider.example${imagePath}`,
    `/core/v1/images/confirm/${imageID}`,
    `${origin}/core/v1/images/confirm/${imageID}`,
    '/logout',
    '/api/user/auth/refresh',
    '/api/user/auth/logout',
    `/api/user/auth/images/confirm/image:${'A'.repeat(64)}`,
    `${imagePath}/`,
    `${imagePath}?approve=true`,
    `${imagePath}#approved`,
    `https://user@local.example${imagePath}`,
    `https://user:password@local.example${imagePath}`,
    `https://@local.example${imagePath}`,
    `https://:@local.example${imagePath}`,
    `https://local.example\t${imagePath}`,
    String.raw`https://local.example\junk\..${imagePath}`,
    `/api/user/auth/images/confirm/../confirm/${imageID}`,
    `${origin}/api/user/auth/images/confirm/%2e%2e/confirm/${imageID}`,
    `/api/user/auth/./images/confirm/${imageID}`,
    `/api/user/auth/images/confirm/other/../${imageID}`,
    'javascript:alert(1)',
  ]) {
    assert.equal(imageConfirmationURL({ confirmation_url: url }, origin), null)
  }
})

test('only a completed and settled native receipt yields image output', () => {
  const receipt = {
    operation_id: 'image-operation',
    state: 'completed',
    charged_credits: 18,
    billing_pending: false,
    output: { data: [{ b64_json: 'cG5n' }] },
  }
  assert.equal(settledImageSchema.parse(receipt).charged_credits, 18)
  for (const invalid of [
    { ...receipt, billing_pending: true },
    { ...receipt, state: 'unknown' },
    { ...receipt, charged_credits: null },
    { ...receipt, charged_credits: -1 },
  ]) {
    assert.equal(settledImageSchema.safeParse(invalid).success, false)
  }
})

test('reference image metadata freezes original bytes in order and rejects limits', async () => {
  const first = new File(['abc'], 'first.png', { type: 'image/png' })
  const second = new File(['def'], 'second.webp', { type: 'image/webp' })
  const metadata = await referenceImageMetadata([first, second])
  assert.deepEqual(metadata[0], {
    media_type: 'image/png',
    byte_length: 3,
    sha256: 'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad',
  })
  assert.equal(metadata[1].media_type, 'image/webp')
  await assert.rejects(
    referenceImageMetadata([
      new File(['a'], 'x.svg', { type: 'image/svg+xml' }),
    ])
  )
  await assert.rejects(
    referenceImageMetadata(Array.from({ length: 9 }, () => first))
  )
  await assert.rejects(
    referenceImageMetadata([
      new File([new Uint8Array(9 * 1024 * 1024)], 'a.png', {
        type: 'image/png',
      }),
      new File([new Uint8Array(9 * 1024 * 1024)], 'b.png', {
        type: 'image/png',
      }),
    ])
  )
})
