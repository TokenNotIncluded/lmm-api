/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { createInstance } from 'i18next'
import { renderToStaticMarkup } from 'react-dom/server'
import { I18nextProvider } from 'react-i18next'

import { usageLogSchema } from '../../data/schema'
import type { LogOtherData } from '../../types'
import { LogUsageDisplay } from '../log-usage-display'

const i18n = createInstance()
await i18n.init({
  lng: 'en',
  fallbackLng: 'en',
  resources: { en: { translation: {} } },
})
const log = usageLogSchema.parse({
  id: 1,
  user_id: 1,
  created_at: 1,
  type: 2,
  content: 'Completed',
  prompt_tokens: 12000,
  completion_tokens: 250,
  token_usage_available: true,
})
const render = (other: LogOtherData | null, tokenUsageAvailable = true) =>
  renderToStaticMarkup(
    <I18nextProvider i18n={i18n}>
      <LogUsageDisplay
        log={{ ...log, token_usage_available: tokenUsageAvailable }}
        other={other}
      />
    </I18nextProvider>
  )

test('12-hour cache display preserves measured zero and its own subset count', () => {
  assert.match(
    render({ cache_creation_tokens: 7000, cache_creation_tokens_12h: 0 }),
    /12-hour cache writes: 0<\/span>/
  )
  assert.match(
    render({ cache_creation_tokens: 7000, cache_creation_tokens_12h: 4000 }),
    new RegExp(
      `12-hour cache writes: ${Number(4000).toLocaleString()}<\\/span>`
    )
  )
})

test('12-hour cache display keeps unknown evidence distinct from zero and historical absence', () => {
  for (const other of [
    { cache_creation_tokens: 7000, cache_creation_tokens_12h: null },
    { cache_creation_tokens: 7000, cache_creation_tokens_12h: 7001 },
    { cache_creation_tokens_12h: 0 },
    {
      cache_creation_tokens: 7000,
      cache_creation_tokens_12h: 0,
      token_usage_available: false,
    },
  ]) {
    const html = render(other)
    assert.match(html, /12-hour cache writes: Unknown<\/span>/)
    assert.doesNotMatch(html, /12-hour cache writes: 0<\/span>/)
  }
  assert.doesNotMatch(
    render({ cache_creation_tokens: 7000 }),
    /12-hour cache writes/
  )
  assert.doesNotMatch(render(null), /12-hour cache writes/)
})

test('unavailable primary token usage also makes a populated 12-hour cache quantity unknown', () => {
  for (const cache_creation_tokens_12h of [0, 4000]) {
    const html = render(
      { cache_creation_tokens: 7000, cache_creation_tokens_12h },
      false
    )
    assert.match(html, /— \/ —/)
    assert.match(html, /12-hour cache writes: Unknown<\/span>/)
    assert.doesNotMatch(html, /12-hour cache writes: [0-9]/)
  }
})
