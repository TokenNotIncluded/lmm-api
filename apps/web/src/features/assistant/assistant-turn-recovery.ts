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
import { z } from 'zod'

import {
  AssistantStreamError,
  assistantAbortReason,
} from './assistant-ai-stream'

const pendingTurn = z
  .object({
    success: z.literal(false),
    code: z.enum([
      'ASSISTANT_OPERATION_PENDING',
      'ASSISTANT_HUMAN_SUPPORT_PENDING',
    ]),
    data: z
      .object({
        conversation_id: z
          .number()
          .int()
          .positive()
          .max(Number.MAX_SAFE_INTEGER),
        client_turn_id: z.string().regex(/^[A-Za-z0-9][A-Za-z0-9_.:-]{0,63}$/),
        state: z.enum([
          'new',
          'model_running',
          'tools_pending',
          'unknown',
          'human_support_pending',
        ]),
        text: z.string().max(32768).optional(),
        gift: z.unknown().optional(),
      })
      .strict(),
  })
  .strict()
  .refine(
    ({ code, data }) =>
      (code === 'ASSISTANT_HUMAN_SUPPORT_PENDING') ===
      (data.state === 'human_support_pending')
  )

const finalReference = z.object({
  lmm_assistant_history: z.object({
    conversation_id: z.number().int().positive(),
    client_turn_id: z.string(),
  }),
})

export class AssistantTurnContinuationError extends AssistantStreamError {
  constructor(
    readonly conversationId: number,
    readonly clientTurnId: string,
    readonly code: string
  ) {
    super(409, { code, retryable: false }, code, false)
  }
}

function pause(signal?: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    if (signal?.aborted) {
      reject(assistantAbortReason(signal))
      return
    }
    const abort = () => {
      clearTimeout(timer)
      reject(signal ? assistantAbortReason(signal) : new Error('Aborted'))
    }
    const timer = setTimeout(() => {
      signal?.removeEventListener('abort', abort)
      resolve()
    }, 250)
    signal?.addEventListener('abort', abort, { once: true })
  })
}

// Only the original server-issued turn may continue. An uncertain operation is
// inspected with GET; a continuation command never supplies new text or grants.
export async function continueAssistantResponse(
  initial: Response,
  expectedTurnId: string | undefined,
  request: (method: 'GET' | 'POST', path: string) => Promise<Response>,
  signal?: AbortSignal
): Promise<Response> {
  let response = initial
  let conversationId: number | undefined
  for (let attempt = 0; response.status === 202; attempt += 1) {
    const parsed = pendingTurn.safeParse(
      await response
        .clone()
        .json()
        .catch(() => null)
    )
    if (
      !parsed.success ||
      !expectedTurnId ||
      parsed.data.data.client_turn_id !== expectedTurnId ||
      (conversationId !== undefined &&
        parsed.data.data.conversation_id !== conversationId)
    ) {
      throw new AssistantStreamError(
        502,
        { retryable: false },
        'Assistant returned an invalid turn receipt',
        false
      )
    }
    const turn = parsed.data.data
    conversationId = turn.conversation_id
    if (turn.state === 'human_support_pending') {
      throw new AssistantTurnContinuationError(
        conversationId,
        expectedTurnId,
        'ASSISTANT_HUMAN_SUPPORT_PENDING'
      )
    }
    if (attempt >= 79) {
      throw new AssistantTurnContinuationError(
        conversationId,
        expectedTurnId,
        'ASSISTANT_OPERATION_PENDING'
      )
    }
    await pause(signal)
    const path = `/api/assistant/conversations/${conversationId}/turns/${encodeURIComponent(expectedTurnId)}`
    response =
      turn.state === 'unknown'
        ? await request('GET', path)
        : await request('POST', `${path}/resume`)
  }
  if (conversationId !== undefined && response.status === 200) {
    const final = finalReference.safeParse(
      await response
        .clone()
        .json()
        .catch(() => null)
    )
    if (
      !final.success ||
      final.data.lmm_assistant_history.conversation_id !== conversationId ||
      final.data.lmm_assistant_history.client_turn_id !== expectedTurnId
    ) {
      throw new AssistantStreamError(
        502,
        { retryable: false },
        'Assistant returned a different turn receipt',
        false
      )
    }
  }
  return response
}
