// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
import assert from 'node:assert/strict'
import { afterEach, test } from 'node:test'

import type { AxiosRequestConfig } from 'axios'

import { api } from '@/lib/api'

import { cancelChallenge } from './api'
import {
  bountyOperationsSnapshot,
  completeBountyOperation,
  settleBountyOperation,
} from './native-operation'

const originalPost = api.post
const originalGet = api.get
function replace(
  name: 'post' | 'get',
  handler: (
    url: string,
    data?: unknown,
    config?: AxiosRequestConfig
  ) => Promise<unknown>
) {
  Object.defineProperty(api, name, {
    value: handler,
    configurable: true,
    writable: true,
  })
}
afterEach(() => {
  api.post = originalPost
  api.get = originalGet
  for (const operation of bountyOperationsSnapshot()) {
    completeBountyOperation(operation.id)
  }
})
const tick = () => new Promise<void>((resolve) => setTimeout(resolve, 0))

test('202 waits for original confirmation and restored business result, never posts another action', async () => {
  const posts: string[] = []
  const gets: string[] = []
  let operationID = ''
  replace(
    'post',
    async (url: string, _data: unknown, config?: AxiosRequestConfig) => {
      posts.push(url)
      const headers = config?.headers
      assert.equal(typeof headers?.['X-Lmm-Operation-ID'], 'string')
      operationID = String(headers?.['X-Lmm-Operation-ID'])
      assert.equal(headers?.['Idempotency-Key'], operationID)
      return {
        status: 202,
        data: {
          success: true,
          data: {
            state: 'awaiting_confirmation',
            operation_id: operationID,
            confirmation_url: '/core/v1/bounties/confirm/frozen-confirmation',
          },
        },
      }
    }
  )
  replace('get', async (url: string) => {
    gets.push(url)
    return {
      status: 200,
      data: { success: true, data: { id: 'claim_hash', status: 'cancelled' } },
    }
  })
  let resolved = false
  const action = cancelChallenge('claim_hash').then((value) => {
    resolved = true
    return value
  })
  await tick()
  assert.equal(resolved, false)
  assert.deepEqual(posts, [
    '/api/open-source-bounties/challenges/claim_hash/cancel',
  ])
  assert.deepEqual(gets, [])
  assert.equal(bountyOperationsSnapshot()[0]?.id, operationID)
  assert.equal(settleBountyOperation(operationID), true)
  const result = await action
  assert.equal(result.id, 'claim_hash')
  assert.deepEqual(gets, [
    `/api/open-source-bounties/operations/${operationID}/result`,
  ])
  assert.equal(posts.length, 1)
  assert.equal(bountyOperationsSnapshot().length, 0)
})

test('lost HTTP response retains exact operation key and waits for recovery instead of resubmitting', async () => {
  let calls = 0
  let originalID = ''
  replace(
    'post',
    async (_url: string, _data: unknown, config?: AxiosRequestConfig) => {
      calls++
      originalID = String(config?.headers?.['X-Lmm-Operation-ID'])
      throw new Error('connection closed after remote commit')
    }
  )
  replace('get', async (url: string) => {
    assert.equal(
      url,
      `/api/open-source-bounties/operations/${originalID}/result`
    )
    return {
      status: 200,
      data: { success: true, data: { id: 'claim_hash', status: 'cancelled' } },
    }
  })
  const action = cancelChallenge('claim_hash')
  await tick()
  assert.equal(bountyOperationsSnapshot()[0]?.id, originalID)
  assert.equal(bountyOperationsSnapshot()[0]?.phase, 'unknown')
  assert.equal(calls, 1)
  settleBountyOperation(originalID)
  await action
  assert.equal(calls, 1)
  assert.equal(bountyOperationsSnapshot().length, 0)
})
