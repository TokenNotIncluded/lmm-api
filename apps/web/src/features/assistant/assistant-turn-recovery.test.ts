/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program. If not, see <https://www.gnu.org/licenses/>.
*/
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { AssistantStreamError } from './assistant-ai-stream'
import {
  AssistantTurnContinuationError,
  continueAssistantResponse,
} from './assistant-turn-recovery'

function pending(state = 'tools_pending', turn = 'original') {
  return Response.json(
    {
      success: false,
      code:
        state === 'human_support_pending'
          ? 'ASSISTANT_HUMAN_SUPPORT_PENDING'
          : 'ASSISTANT_OPERATION_PENDING',
      data: { conversation_id: 42, client_turn_id: turn, state },
    },
    { status: 202 }
  )
}
function completed(turn = 'original') {
  return Response.json({
    success: true,
    choices: [{ message: { content: 'Persisted answer' } }],
    lmm_assistant_history: { conversation_id: 42, client_turn_id: turn },
  })
}

test('pending continuation posts only to the original turn resume endpoint', async () => {
  const requests: Array<{ method: string; path: string }> = []
  const response = await continueAssistantResponse(
    pending(),
    'original',
    async (method, path) => {
      requests.push({ method, path })
      return completed()
    }
  )
  assert.equal(response.status, 200)
  assert.deepEqual(requests, [
    {
      method: 'POST',
      path: '/api/assistant/conversations/42/turns/original/resume',
    },
  ])
})

test('unknown outcomes only inspect the original operation', async () => {
  const requests: string[] = []
  await continueAssistantResponse(
    pending('unknown'),
    'original',
    async (method, path) => {
      requests.push(`${method} ${path}`)
      return requests.length === 1 ? pending('unknown') : completed()
    }
  )
  assert.deepEqual(requests, [
    'GET /api/assistant/conversations/42/turns/original',
    'GET /api/assistant/conversations/42/turns/original',
  ])
})

test('human support stops continuation without another request', async () => {
  let calls = 0
  await assert.rejects(
    continueAssistantResponse(
      pending('human_support_pending'),
      'original',
      async () => {
        calls += 1
        return completed()
      }
    ),
    (error: unknown) =>
      error instanceof AssistantTurnContinuationError &&
      error.code === 'ASSISTANT_HUMAN_SUPPORT_PENDING' &&
      error.conversationId === 42 &&
      error.retryable === false
  )
  assert.equal(calls, 0)
})

test('mismatched turn receipts never authorize continuation or a new chat', async () => {
  let calls = 0
  await assert.rejects(
    continueAssistantResponse(
      pending('new', 'different'),
      'original',
      async () => {
        calls += 1
        return completed()
      }
    ),
    AssistantStreamError
  )
  assert.equal(calls, 0)
  await assert.rejects(
    continueAssistantResponse(pending(), 'original', async () =>
      completed('different')
    ),
    AssistantStreamError
  )
})

test('cancellation stops waiting before any continuation request', async () => {
  let calls = 0
  await assert.rejects(
    continueAssistantResponse(
      pending(),
      'original',
      async () => {
        calls += 1
        return completed()
      },
      AbortSignal.abort()
    ),
    { name: 'AbortError' }
  )
  assert.equal(calls, 0)
})
