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

import { buildCCSwitchProviderURL } from './cc-switch-deep-link'

describe('CC Switch deep links', () => {
  test('builds a URL-encoded Claude provider import link', () => {
    assert.equal(
      buildCCSwitchProviderURL({
        app: 'claude',
        name: 'LMM Claude',
        endpoint: 'https://api.lmm.best',
        apiKey: 'sk-secret',
        models: { model: 'deepseek-v4-flash', haikuModel: '' },
        homepage: 'https://api.lmm.best',
        enabled: true,
      }),
      'ccswitch://v1/import?resource=provider&app=claude&name=LMM+Claude&endpoint=https%3A%2F%2Fapi.lmm.best&apiKey=sk-secret&model=deepseek-v4-flash&homepage=https%3A%2F%2Fapi.lmm.best&enabled=true'
    )
  })
})
