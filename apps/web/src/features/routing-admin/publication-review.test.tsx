import assert from 'node:assert/strict'
import { test } from 'node:test'

import { createInstance } from 'i18next'
import { renderToStaticMarkup } from 'react-dom/server'
import { I18nextProvider } from 'react-i18next'

import type { Preview } from './contract'
import { preview } from './fixtures.test-helper'
import { PublicationReview } from './publication-review'
async function render(value: Preview) {
  const i18n = createInstance()
  await i18n.init({
    lng: 'en',
    resources: { en: { translation: {} } },
    interpolation: { escapeValue: false },
  })
  return renderToStaticMarkup(
    <I18nextProvider i18n={i18n}>
      <PublicationReview
        preview={value}
        busy={false}
        onConfirm={() => {}}
        onCancel={() => {}}
      />
    </I18nextProvider>
  )
}
test('review displays exact current/source/final amounts, zero and frozen FX', async () => {
  const html = await render(preview)
  for (const text of [
    'Current price',
    'Source price',
    'Final price',
    '7 USD',
    '1 USD',
    '0 USD',
    'Frozen site exchange rate',
    'Confirm publication',
  ])
    assert.ok(html.includes(text), text)
  assert.ok(!html.includes('NaN'))
  assert.ok(!html.includes('undefined'))
})
test('priority review shows affected target and both priorities', async () => {
  const html = await render({
    ...preview,
    kind: 'priorities',
    exchange_rate: null,
    next: { ...preview.next, pricing: '1' },
    changes: [
      { route_id: '1', target_id: '2', upstream_id: '3', before: 10, after: 0 },
    ],
  })
  for (const text of [
    '#2',
    '#3',
    'Current priority',
    'Final priority',
    '>10<',
    '>0<',
  ])
    assert.ok(html.includes(text), text)
})
test('expired preview disables publication', async () => {
  const html = await render({ ...preview, expires_at: '1' })
  assert.ok(html.includes('This preview expired. Create a new preview.'))
  assert.match(html, /<button[^>]*disabled[^>]*>Confirm publication<\/button>/)
})
