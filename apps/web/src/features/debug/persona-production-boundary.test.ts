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

const config = readFileSync(
  new URL('../../../rsbuild.config.ts', import.meta.url),
  'utf8'
)
const productionEntry = readFileSync(
  new URL('../../../src/main.tsx', import.meta.url),
  'utf8'
)

describe('persona debug production boundary', () => {
  test('requires an explicit non-production server entry', () => {
    assert.match(
      config,
      /personaDebugEnabled\s*=\s*!isProd\s*&&\s*process\.env\.LMM_ENABLE_PERSONA_DEBUG\s*===\s*'1'/
    )
    assert.match(
      config,
      /personaDebugEnabled\s*\?\s*'\.\/src\/debug-main\.tsx'\s*:\s*'\.\/src\/main\.tsx'/
    )
    assert.match(config, /personaDebugEnabled\s*\?\s*\{\}\s*:/)
    assert.match(
      config,
      /__LMM_PERSONA_DEBUG__:\s*JSON\.stringify\(personaDebugEnabled\)/
    )
  })

  test('keeps fixture installation out of the normal application entry', () => {
    assert.doesNotMatch(productionEntry, /persona-runtime|persona-debug/)
  })
})
