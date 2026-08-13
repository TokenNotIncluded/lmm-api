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

import { getNavLinkKey } from './nav-link-key'

describe('navigation link identity', () => {
  test('uses the route identity instead of the translated title', () => {
    const key = getNavLinkKey({ href: '/pricing', title: 'Model Square' })
    const translatedKey = getNavLinkKey({ href: '/pricing', title: '模型广场' })

    assert.equal(key, 'internal:/pricing')
    assert.equal(translatedKey, key)
  })

  test('keeps internal and external destinations distinct', () => {
    assert.notEqual(
      getNavLinkKey({ href: '/docs', external: false, title: 'Docs' }),
      getNavLinkKey({ href: '/docs', external: true, title: 'Docs' })
    )
  })
})
