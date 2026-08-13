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
import { test } from 'node:test'

import {
  CHAT_IFRAME_ALLOW,
  CHAT_IFRAME_REFERRER_POLICY,
  CHAT_IFRAME_SANDBOX,
} from './$chatId'

test('chat embeds keep the minimum interactive iframe permissions', () => {
  assert.deepEqual(CHAT_IFRAME_SANDBOX.split(' '), [
    'allow-scripts',
    'allow-forms',
    'allow-popups',
    'allow-presentation',
  ])
  assert.ok(!CHAT_IFRAME_SANDBOX.includes('allow-same-origin'))
  assert.ok(!CHAT_IFRAME_SANDBOX.includes('allow-top-navigation'))
  assert.ok(!CHAT_IFRAME_SANDBOX.includes('allow-popups-to-escape-sandbox'))
  assert.equal(CHAT_IFRAME_REFERRER_POLICY, 'no-referrer')
  assert.equal(CHAT_IFRAME_ALLOW, 'camera; microphone')
})
