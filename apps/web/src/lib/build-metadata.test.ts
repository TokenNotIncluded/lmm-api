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
import { after, before, test } from 'node:test'

const previousVersion = process.env.VITE_REACT_APP_VERSION

before(() => {
  process.env.VITE_REACT_APP_VERSION = '0.1.1-test-build'
})

after(() => {
  if (previousVersion === undefined) {
    delete process.env.VITE_REACT_APP_VERSION
    return
  }
  process.env.VITE_REACT_APP_VERSION = previousVersion
})

test('build metadata exposes the release revision from the build environment', async () => {
  const { getBuildRevision, getBuildVersion } = await import('./build-metadata')

  assert.equal(getBuildRevision(), 'rv.0.1.1-test-build.2k6e8r7p')
  assert.equal(getBuildVersion(), '0.1.1-test-build')
})
