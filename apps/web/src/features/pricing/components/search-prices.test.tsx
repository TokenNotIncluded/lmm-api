/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { createInstance } from 'i18next'
import { renderToStaticMarkup } from 'react-dom/server'
import { I18nextProvider } from 'react-i18next'

import en from '@/i18n/locales/en.json'
import zh from '@/i18n/locales/zh.json'

import { nativeSearchPriceSchema } from '../catalog-contract'
import { SearchPrices } from './search-prices'

const price = {
  available: true,
  provider: 'gemini_grounding_v1' as const,
  unit: 'query' as const,
  usd_per_unit: { numerator: '1', denominator: '3' },
  x_posts_fetched_usd_per_unit: null,
  x_users_fetched_usd_per_unit: null,
}

test('search catalog rejects incompatible units, currencies and provider dimensions', () => {
  assert.ok(nativeSearchPriceSchema.safeParse(price).success)
  for (const invalid of [
    { ...price, unit: 'call' },
    { ...price, provider: 'unknown' },
    { ...price, usd_per_unit: { numerator: '1', denominator: '0' } },
    {
      ...price,
      x_posts_fetched_usd_per_unit: { numerator: '1', denominator: '1' },
    },
    { ...price, currency: 'CNY' },
  ])
    assert.ok(!nativeSearchPriceSchema.safeParse(invalid).success)
})

test('search price renders exact rates, unit, availability, group filtering and language', async () => {
  const i18n = createInstance()
  await i18n.init({
    lng: 'en',
    fallbackLng: 'en',
    resources: { en, zh },
    interpolation: { escapeValue: false },
  })
  const model = {
    id: 1,
    model_name: 'synthetic',
    quota_type: 0,
    enable_groups: ['default'],
    group_search_prices: { default: price },
  }
  const render = () =>
    renderToStaticMarkup(
      <I18nextProvider i18n={i18n}>
        <SearchPrices model={model} />
      </I18nextProvider>
    )
  assert.ok(render().includes('Search queries'))
  assert.ok(render().includes('1/3 USD'))
  assert.ok(!render().includes('Search calls'))
  model.group_search_prices.default = { ...price, available: false }
  assert.ok(render().includes('Search is not enabled for this route'))
  await i18n.changeLanguage('zh')
  assert.ok(render().includes('搜索查询数'))
  model.enable_groups = []
  assert.equal(render(), '')
})
