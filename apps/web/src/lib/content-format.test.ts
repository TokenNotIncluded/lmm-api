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
  isHttpUrl,
  isSafeHttpUrl,
  isSafePublicHttpUrl,
  isSafeResourceUrl,
} from './content-format'

describe('safe URL helpers', () => {
  test('accepts http and https URLs without credentials', () => {
    assert.equal(isSafeHttpUrl('https://example.com/path'), true)
    assert.equal(isHttpUrl('http://localhost:3000/setup'), true)
  })

  test('rejects non-http schemes and embedded credentials', () => {
    assert.equal(isSafeHttpUrl('javascript:alert(1)'), false)
    assert.equal(isSafeHttpUrl('data:text/html,hi'), false)
    assert.equal(isSafeHttpUrl('https://user:pass@example.com/'), false)
    assert.equal(isSafeHttpUrl('/relative'), false)
  })

  test('allows public HTTPS document URLs and rejects local destinations', () => {
    for (const value of [
      'https://example.com/legal',
      'https://8.8.8.8/legal',
    ]) {
      assert.equal(isSafePublicHttpUrl(value), true, value)
    }

    for (const value of [
      'http://example.com/legal',
      'https://localhost/legal',
      'https://admin.localhost/legal',
      'https://admin.local/legal',
      'https://admin.internal/legal',
      'https://127.0.0.1/legal',
      'https://2130706433/legal',
      'https://10.0.0.1/legal',
      'https://172.16.0.1/legal',
      'https://192.168.1.1/legal',
      'https://169.254.169.254/legal',
      'https://100.64.0.1/legal',
      'https://192.0.2.1/legal',
      'https://[::1]/legal',
      'https://[::ffff:127.0.0.1]/legal',
      'https://[fc00::1]/legal',
      'https://[fe80::1]/legal',
      'https://[ff02::1]/legal',
      'https://[2001:db8::1]/legal',
      'https://[3ffe::1]/legal',
    ]) {
      assert.equal(isSafePublicHttpUrl(value), false, value)
    }

    assert.equal(isSafePublicHttpUrl('/legal'), false)
  })

  test('allows same-origin resource paths and blocks protocol-relative URLs', () => {
    assert.equal(isSafeResourceUrl('/logo.svg'), true)
    assert.equal(isSafeResourceUrl('//evil.example/logo.svg'), false)
    assert.equal(isSafeResourceUrl('javascript:alert(1)'), false)
  })
})
