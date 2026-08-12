/*
Copyright (C) 2023-2026 QuantumNous

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

For commercial licensing, please contact support@quantumnous.com
*/
import assert from 'node:assert/strict'
import { describe, test } from 'node:test'

import {
  buildAssistantConversation,
  parseAssistantAction,
  parseAssistantIntent,
  parseAssistantReply,
  type AssistantChatMessage,
} from './api'

describe('assistant response parsing', () => {
  test('extracts the first assistant message', () => {
    assert.equal(
      parseAssistantReply({
        choices: [{ message: { content: '  Use /v1 as the Base URL.  ' } }],
      }),
      'Use /v1 as the Base URL.'
    )
  })

  test('rejects empty or malformed upstream responses', () => {
    assert.throws(
      () => parseAssistantReply({ error: { message: 'model unavailable' } }),
      /model unavailable/
    )
    assert.throws(
      () => parseAssistantReply({ choices: [] }),
      /Assistant returned no answer/
    )
  })

  test('accepts only known assistant intent headers', () => {
    assert.equal(parseAssistantIntent(' client_setup '), 'client_setup')
    assert.equal(parseAssistantIntent('HUMAN_SUPPORT'), 'human_support')
    assert.equal(parseAssistantIntent('unknown-intent'), undefined)
    assert.equal(parseAssistantIntent(undefined), undefined)
  })

  test('accepts only complete L1 recommendation actions', () => {
    assert.deepEqual(
      parseAssistantAction({
        type: 'l1_recommendation',
        user_statement: '  I am building an internal coding tool. ',
        recommendation: ' Recommend L1 because the use case is concrete. ',
        confirmation_token: ' confirmation-token ',
      }),
      {
        type: 'l1_recommendation',
        user_statement: 'I am building an internal coding tool.',
        recommendation: 'Recommend L1 because the use case is concrete.',
        confirmation_token: 'confirmation-token',
      }
    )
    assert.equal(
      parseAssistantAction({
        type: 'l1_recommendation',
        user_statement: '',
        recommendation: 'missing statement',
      }),
      undefined
    )
    assert.equal(parseAssistantAction({ type: 'open_wallet' }), undefined)
  })

  test('accepts exact administrator previews and keeps the confirmation token', () => {
    assert.deepEqual(
      parseAssistantAction({
        type: 'admin_config_change',
        confirmation_token: ' admin-token ',
        requires_confirmation: true,
        expires_in_seconds: 600,
        changes: [
          {
            key: 'AssistantModel',
            label: 'Default assistant model ID',
            old_value: 'old-model',
            new_value: 'new-model',
          },
        ],
      }),
      {
        type: 'admin_config_change',
        confirmation_token: 'admin-token',
        requires_confirmation: true,
        expires_in_seconds: 600,
        changes: [
          {
            key: 'AssistantModel',
            label: 'Default assistant model ID',
            old_value: 'old-model',
            new_value: 'new-model',
          },
        ],
      }
    )
    assert.equal(
      parseAssistantAction({
        type: 'admin_config_change',
        confirmation_token: 'admin-token',
        requires_confirmation: true,
        expires_in_seconds: 600,
        changes: [{ key: 'AssistantModel' }],
      }),
      undefined
    )
    assert.deepEqual(
      parseAssistantAction({
        type: 'admin_pricing_change',
        confirmation_token: ' pricing-token ',
        requires_confirmation: true,
        expires_in_seconds: 600,
        pricing: {
          model_id: 'deepseek-v4-flash',
          old: { mode: 'ratio', value: 1 },
          next: { mode: 'ratio', value: 0.8 },
        },
      }),
      {
        type: 'admin_pricing_change',
        confirmation_token: 'pricing-token',
        requires_confirmation: true,
        expires_in_seconds: 600,
        pricing: {
          model_id: 'deepseek-v4-flash',
          old: { mode: 'ratio', value: 1 },
          next: { mode: 'ratio', value: 0.8 },
        },
      }
    )
  })
})

describe('assistant conversation context', () => {
  test('keeps recent multi-turn context and always starts with a user message', () => {
    const history: AssistantChatMessage[] = [
      { role: 'assistant', content: 'orphaned model reply' },
      { role: 'user', content: 'How do I configure Claude Code?' },
      { role: 'assistant', content: 'Choose your operating system.' },
    ]

    assert.deepEqual(
      buildAssistantConversation(history, '  What about Windows?  '),
      [
        { role: 'user', content: 'How do I configure Claude Code?' },
        { role: 'assistant', content: 'Choose your operating system.' },
        { role: 'user', content: 'What about Windows?' },
      ]
    )
  })

  test('bounds context by message count without losing the latest question', () => {
    const history: AssistantChatMessage[] = Array.from(
      { length: 20 },
      (_, index) => ({
        role: index % 2 === 0 ? ('user' as const) : ('assistant' as const),
        content: `message-${index}`,
      })
    )

    const conversation = buildAssistantConversation(history, 'latest')
    assert.ok(conversation.length <= 12)
    assert.equal(conversation[0]?.role, 'user')
    assert.deepEqual(conversation.at(-1), {
      role: 'user',
      content: 'latest',
    })
    assert.equal(
      conversation.some((message) => message.content === 'message-0'),
      false
    )
    assert.equal(
      conversation.some((message) => message.content === 'message-19'),
      true
    )
  })

  test('rejects an oversized latest message before making a request', () => {
    assert.throws(
      () => buildAssistantConversation([], '问'.repeat(4001)),
      /between 1 and 4000 characters/
    )
  })
})
