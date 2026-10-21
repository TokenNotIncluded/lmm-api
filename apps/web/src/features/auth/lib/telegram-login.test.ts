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
  parseTelegramMFAChallenge,
  parseTelegramMFARequirement,
  pickTelegramAuthorization,
  telegramFlowSchema,
} from './telegram-login'

const signature = 'aB'.repeat(32)

describe('Telegram login authorization', () => {
  test('keeps only fields signed by the Telegram login contract', () => {
    assert.deepEqual(
      pickTelegramAuthorization({
        id: 12345,
        first_name: 'Test',
        last_name: 'User',
        username: 'test_user',
        photo_url: 'https://t.me/i/userpic/320/test.jpg',
        auth_date: 1_900_000_000,
        hash: signature,
        lang: 'en',
        admin: true,
        redirect: 'https://attacker.example',
      }),
      {
        id: 12345,
        first_name: 'Test',
        last_name: 'User',
        username: 'test_user',
        photo_url: 'https://t.me/i/userpic/320/test.jpg',
        auth_date: 1_900_000_000,
        hash: signature,
        lang: 'en',
      }
    )
  })

  test('rejects incomplete or structurally invalid callbacks', () => {
    assert.equal(pickTelegramAuthorization(null), null)
    assert.equal(
      pickTelegramAuthorization({ auth_date: 1, hash: 'hash' }),
      null
    )
    assert.equal(
      pickTelegramAuthorization({ id: 1, auth_date: 1, hash: '' }),
      null
    )
    assert.equal(
      pickTelegramAuthorization({ id: {}, auth_date: 1, hash: 'hash' }),
      null
    )
  })

  test('preserves present empty and Unicode signed values without normalizing them', () => {
    const authorization = {
      id: '9223372036854775807',
      auth_date: '1900000000',
      hash: signature,
      first_name: ' e\u0301 ',
      last_name: '',
      username: '',
      photo_url: '',
      lang: '',
    }
    assert.deepEqual(pickTelegramAuthorization(authorization), authorization)
    assert.equal(
      pickTelegramAuthorization({ ...authorization, hash: ` ${signature}` }),
      null
    )
  })

  test('rejects lossy or noncanonical numeric fields instead of changing their signed text', () => {
    for (const id of [
      0,
      -1,
      1.5,
      Number.MAX_SAFE_INTEGER + 1,
      '01',
      '+1',
      ' 1',
      '1 ',
      '9223372036854775808',
      'not-an-id',
    ]) {
      assert.equal(
        pickTelegramAuthorization({
          id,
          auth_date: 1_900_000_000,
          hash: signature,
        }),
        null
      )
    }
  })

  test('requires the native flow and shared MFA challenge shapes', () => {
    const expiry = 1_900_000_000
    const flow = { flow_token: `lmmt_${'a'.repeat(64)}`, expires_at: expiry }
    assert.deepEqual(telegramFlowSchema.parse(flow), flow)
    assert.equal(
      telegramFlowSchema.safeParse({ ...flow, flow_token: 'legacy-token' })
        .success,
      false
    )
    const challenge = {
      require_2fa: true,
      flow_token: `lmmf_${'b'.repeat(64)}`,
      expires_at: expiry,
    }
    assert.deepEqual(parseTelegramMFAChallenge(challenge), challenge)
    assert.equal(
      parseTelegramMFAChallenge({ ...challenge, expires_at: null }),
      null
    )
    assert.equal(
      parseTelegramMFAChallenge({ ...challenge, require_2fa: false }),
      null
    )
    assert.equal(parseTelegramMFARequirement({}), false)
    assert.equal(parseTelegramMFARequirement(challenge), true)
    assert.throws(() => parseTelegramMFARequirement({ require_2fa: 'true' }))
  })
})
