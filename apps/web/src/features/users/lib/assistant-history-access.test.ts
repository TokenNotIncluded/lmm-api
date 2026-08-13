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

import { canViewUserAssistantHistory } from './assistant-history-access'

describe('assistant conversation visibility', () => {
  const user = { id: 1, role: 1 }
  const admin = { id: 5, role: 10 }
  const peerAdmin = { id: 6, role: 10 }
  const root = { id: 7, role: 100 }

  test('keeps ordinary users scoped to themselves', () => {
    assert.equal(canViewUserAssistantHistory(user, user), true)
    assert.equal(canViewUserAssistantHistory(user, { id: 2, role: 1 }), false)
  })

  test('allows only strictly higher administrator roles', () => {
    assert.equal(canViewUserAssistantHistory(admin, user), true)
    assert.equal(canViewUserAssistantHistory(admin, peerAdmin), false)
    assert.equal(canViewUserAssistantHistory(admin, root), false)
    assert.equal(canViewUserAssistantHistory(root, admin), true)
    assert.equal(canViewUserAssistantHistory(root, root), true)
  })
})
