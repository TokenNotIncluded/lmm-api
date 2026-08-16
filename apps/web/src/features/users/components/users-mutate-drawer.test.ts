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
import { readFileSync } from 'node:fs'
import { describe, test } from 'node:test'

const source = readFileSync(
  new URL('./users-mutate-drawer.tsx', import.meta.url),
  'utf8'
)

describe('UsersMutateDrawer query loading', () => {
  test('defers drawer metadata queries until the drawer is open', () => {
    assert.match(
      source,
      /queryKey:\s*\['groups'\][\s\S]*?queryFn:\s*getGroups,\s*enabled:\s*open/
    )
    assert.match(
      source,
      /queryKey:\s*\['admin-permission-catalog'\][\s\S]*?queryFn:\s*getPermissionCatalog,\s*enabled:\s*open/
    )
  })
})
