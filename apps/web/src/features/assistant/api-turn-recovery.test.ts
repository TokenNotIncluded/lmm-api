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

import { sendAssistantMessage } from './api'

test('the existing streaming entry point resumes JSON turns with an empty command', async () => {
  const calls: Array<{
    url: string
    method: string | undefined
    body: BodyInit | null | undefined
  }> = []
  const original = Object.getOwnPropertyDescriptor(globalThis, 'fetch')
  assert.ok(original)
  Object.defineProperty(globalThis, 'fetch', {
    ...original,
    value: async (input: RequestInfo | URL, init?: RequestInit) => {
      calls.push({ url: String(input), method: init?.method, body: init?.body })
      return calls.length === 1
        ? Response.json(
            {
              success: false,
              code: 'ASSISTANT_OPERATION_PENDING',
              data: {
                conversation_id: 42,
                client_turn_id: 'original-turn',
                state: 'tools_pending',
              },
            },
            { status: 202 }
          )
        : Response.json({
            success: true,
            choices: [{ message: { content: 'Original persisted answer' } }],
            lmm_assistant_history: {
              conversation_id: 42,
              client_turn_id: 'original-turn',
            },
          })
    },
  })
  try {
    const reply = await sendAssistantMessage(
      'Original message',
      [],
      undefined,
      undefined,
      { onDelta: () => undefined },
      undefined,
      false,
      'original-turn'
    )
    assert.equal(reply.content, 'Original persisted answer')
    assert.equal(reply.conversationId, 42)
    assert.equal(calls.length, 2)
    assert.equal(calls[0]?.url, '/api/assistant/chat')
    assert.deepEqual(calls[1], {
      url: '/api/assistant/conversations/42/turns/original-turn/resume',
      method: 'POST',
      body: '{}',
    })
  } finally {
    Object.defineProperty(globalThis, 'fetch', original)
  }
})
