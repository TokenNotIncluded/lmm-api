/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { createInstance } from 'i18next'
import { renderToStaticMarkup } from 'react-dom/server'
import { I18nextProvider } from 'react-i18next'

import en from '@/i18n/locales/en.json'
import zh from '@/i18n/locales/zh.json'

import { nativeTokenPriceSchema } from '../catalog-contract'
import { EmbeddingPrices } from './embedding-prices'

const embedding = {
  available: true,
  provider: 'ali_separate_input_v1' as const,
  text_usd_per_million: { numerator: '1', denominator: '3' },
  image_usd_per_million: { numerator: '0', denominator: '1' },
}
const price = {
  currency: 'USD',
  unit: 'million_tokens',
  rate_basis: 'base_group_rate',
  group_multiplier: 1,
  input: null,
  output: null,
  cache_read: null,
  cache_write: null,
  request: null,
  final_cost_depends_on_usage: true,
  embedding,
}

test('embedding catalog requires exact modality prices and excludes misleading scalar prices', () => {
  assert.ok(nativeTokenPriceSchema.safeParse(price).success)
  for (const invalid of [
    { ...price, input: 0 },
    { ...price, output: 0 },
    { ...price, currency: 'CNY' },
    { ...price, embedding: { ...embedding, provider: 'unknown' } },
    {
      ...price,
      embedding: {
        ...embedding,
        image_usd_per_million: { numerator: '1', denominator: '0' },
      },
    },
  ]) {
    assert.ok(!nativeTokenPriceSchema.safeParse(invalid).success)
  }
})

test('embedding prices render both exact rates, availability and authorized groups in each language', async () => {
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
    group_embedding_prices: { default: embedding },
  }
  const render = () =>
    renderToStaticMarkup(
      <I18nextProvider i18n={i18n}>
        <EmbeddingPrices model={model} />
      </I18nextProvider>
    )
  assert.ok(render().includes('1/3 USD'))
  assert.ok(render().includes('0 USD'))
  assert.ok(render().includes('Embedding image and video input tokens'))
  model.group_embedding_prices.default = { ...embedding, available: false }
  assert.ok(render().includes('Embedding is not enabled for this route'))
  await i18n.changeLanguage('zh')
  assert.ok(render().includes('多模态向量化价格'))
  model.enable_groups = []
  assert.equal(render(), '')
})
