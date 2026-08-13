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
  hasAssistantMessageSubstantialMeaning,
  redactAssistantMessageForDisplay,
  redactAssistantMessageForRequest,
} from './assistant-message-safety'

describe('assistant message safety', () => {
  test('redacts common email, phone, and API key formats', () => {
    const result = redactAssistantMessageForDisplay(
      'email alice@example.test, phone +86 138 0013 8000, api_key=sk-live-secret-token-123456',
      '[REDACTED]'
    )

    assert.equal(result.redacted, true)
    assert.doesNotMatch(result.content, /alice@example\.test/)
    assert.doesNotMatch(result.content, /138 0013 8000/)
    assert.doesNotMatch(result.content, /sk-live-secret-token-123456/)
    assert.equal((result.content.match(/\[REDACTED\]/g) ?? []).length, 3)
  })

  test('keeps useful context while recognizing a secret-only message', () => {
    const mixed = redactAssistantMessageForRequest(
      'Use sk-super-secret-123456 to explain the failing request.'
    )
    const secretOnly = redactAssistantMessageForRequest(
      'sk-super-secret-123456'
    )

    assert.equal(mixed.redacted, true)
    assert.match(mixed.content, /\[REDACTED_API_KEY\]/)
    assert.equal(hasAssistantMessageSubstantialMeaning(mixed.content), true)
    assert.equal(secretOnly.redacted, true)
    assert.equal(
      hasAssistantMessageSubstantialMeaning(secretOnly.content),
      false
    )
  })
})
